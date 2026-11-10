//! The `cicada` command line and terminal UI for the Cicada HTTP API (`cicada_axum::actor_api`).
//!
//! It talks to a node only over HTTP, through [`cicada_client::ActorClient`]. The server comes from `--server`,
//! `CICADA_SERVER` or a context of the config file (see [`config`]); [`run`] executes one parsed command.

pub mod cli;
pub mod config;
pub mod output;
pub mod tui;

use std::io::Write;
use std::time::Duration;

use anyhow::{Context as _, bail};
use cicada::SendOptions;
use cicada::api::{BulkRequest, PageView};
use cicada::store::RemoveTypeResult;
use cicada::time::format_instant;
use cicada_client::{ActorClient, CancelOutcome};
use serde_json::json;

use crate::cli::{Cancel, Cli, Command, ConfigCommand, Create, Delete, Describe, Get, Page, Retry, Selection};
use crate::config::{Config, Env, Overrides};
use crate::output::{Format, list, one, structured};

/// `error` and its causes, each cause only when the text so far does not already hold it (a `ClientError` repeats
/// its source in its own message).
pub fn error_text(error: &anyhow::Error) -> String {
    let mut text = error.to_string();
    for cause in error.chain().skip(1).map(ToString::to_string) {
        if !text.contains(&cause) {
            text = format!("{text}: {cause}");
        }
    }
    text
}

/// How long a CLI request may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `cli` with the environment `env`, writing results to `out`. Any error means exit code 1.
pub async fn run(cli: Cli, env: &Env, out: &mut dyn Write) -> anyhow::Result<()> {
    if let Command::Config(command) = cli.command {
        return config_command(command, cli.server, cli.token, env, out);
    }
    let flags = Overrides { context: cli.context, server: cli.server, token: cli.token };
    let target = config::resolve(&flags, env, &Config::load(&config::path(env)?)?)?;
    if let Command::Tui = cli.command {
        return tui::run(target).await;
    }
    let client = target.client(TIMEOUT)?;
    let format = cli.output.unwrap_or(Format::Table);
    match cli.command {
        Command::Get(get) => get_command(&client, get, format, out).await,
        Command::Describe(describe) => {
            describe_command(&client, describe, cli.output.unwrap_or(Format::Yaml), out).await
        }
        Command::Create(Create::Actor { actor, state }) => {
            let created = client.create_actor(&actor, state).await?;
            let verb = if created.created { "created" } else { "exists, left untouched" };
            done(out, format, &format!("actor {} {verb}", created.address), json!(created))
        }
        Command::Delete(Delete::Actor { actor }) => {
            found(client.delete_actor(&actor).await?.then_some(()), || format!("actor {actor}"))?;
            writeln!(out, "actor {actor} deleted").map_err(Into::into)
        }
        Command::Delete(Delete::Type { name }) => match client.remove_type(&name).await? {
            RemoveTypeResult::Removed => writeln!(out, "type {name} removed").map_err(Into::into),
            RemoveTypeResult::Absent => bail!("type {name} not found"),
            RemoveTypeResult::InUse => bail!("type {name} is in use: actors of it exist"),
        },
        Command::Send(args) => {
            let mut options = SendOptions::new();
            options.id = args.id;
            options.correlation_id = args.correlation_id;
            options.headers = args.headers.into_iter().collect();
            if let Some(delay) = args.delay {
                options = options.delay(delay);
            }
            if let Some(at) = args.at {
                options = options.at(at);
            }
            let receipt = client.send_message(&args.actor, &args.message_type, args.payload, options).await?;
            let due = receipt.deliver_at.map(format_instant);
            let text = match (&due, receipt.duplicate) {
                (_, true) => format!("message {} already accepted", receipt.id),
                (Some(due), false) => format!("message {} sent, due {due}", receipt.id),
                (None, false) => format!("message {} sent", receipt.id),
            };
            done(out, format, &text, json!({"id": receipt.id, "deliverAt": due, "duplicate": receipt.duplicate}))
        }
        Command::Cancel(Cancel::Message { id }) => match client.cancel(&id).await? {
            CancelOutcome::Cancelled => writeln!(out, "message {id} cancelled").map_err(Into::into),
            CancelOutcome::Running => bail!("message {id} is running; stop its turn with `cicada cancel turn`"),
            CancelOutcome::AlreadyFinished => bail!("message {id} has already finished"),
            CancelOutcome::NotFound => bail!("message {id} not found"),
        },
        Command::Cancel(Cancel::Messages(selection)) => {
            let response = client.bulk_cancel(&bulk(selection)).await?;
            let more = if response.more { "; more remain, run it again" } else { "" };
            done(out, format, &format!("{} messages cancelled{more}", response.affected), json!(response))
        }
        Command::Cancel(Cancel::Turn { actor }) => {
            if !client.cancel_turn(&actor).await? {
                bail!("no turn is running on actor {actor}");
            }
            writeln!(out, "turn on actor {actor} asked to cancel").map_err(Into::into)
        }
        Command::Retry(Retry::Message { id }) => {
            let receipt = found(client.retry(&id).await?, || format!("message {id}"))?;
            let due = receipt.deliver_at.map(format_instant);
            done(
                out,
                format,
                &format!("message {id} retried as {}", receipt.id),
                json!({"id": receipt.id, "deliverAt": due}),
            )
        }
        Command::Retry(Retry::Messages(selection)) => {
            let response = client.bulk_retry(&bulk(selection)).await?;
            let more = if response.more { "; more remain, run it again" } else { "" };
            done(out, format, &format!("{} messages retried{more}", response.affected), json!(response))
        }
        Command::DeliverNow { id } => {
            let message = found(client.deliver_now(&id).await?, || format!("message {id}"))?;
            done(out, format, &format!("message {id} due now"), json!(message))
        }
        Command::Pause { actor } => {
            found(client.pause_mailbox(&actor).await?.then_some(()), || format!("actor {actor}"))?;
            writeln!(out, "actor {actor} paused").map_err(Into::into)
        }
        Command::Resume { actor } => {
            found(client.resume_mailbox(&actor).await?.then_some(()), || format!("actor {actor}"))?;
            writeln!(out, "actor {actor} resumed").map_err(Into::into)
        }
        Command::Purge { actor } => {
            let purged = found(client.purge_mailbox(&actor).await?, || format!("actor {actor}"))?;
            done(out, format, &format!("{purged} messages of actor {actor} purged"), json!({"purged": purged}))
        }
        Command::Stats { prefix } => one(out, format, &client.stats(&prefix).await?),
        Command::Health => {
            if !client.health().await? {
                bail!("{} is unhealthy: the node is stopping or its store does not answer", target.server);
            }
            writeln!(out, "{} is healthy", target.server).map_err(Into::into)
        }
        Command::Config(_) | Command::Tui => unreachable!("handled above"),
    }
}

async fn get_command(client: &ActorClient, get: Get, format: Format, out: &mut dyn Write) -> anyhow::Result<()> {
    match get {
        Get::Actors { prefix, page } => list(
            out,
            format,
            &pages(page, async |c| Ok(client.list_actors(&prefix, c, Some(page.limit)).await?)).await?,
        ),
        Get::Actor { actor } => one(out, format, &found(client.get_actor(&actor).await?, || format!("actor {actor}"))?),
        Get::Mailbox { actor, page } => {
            let items = pages(page, async |c| {
                found(client.mailbox(&actor, c, Some(page.limit)).await?, || format!("actor {actor}"))
            });
            list(out, format, &items.await?)
        }
        Get::Messages { prefix, page } => list(
            out,
            format,
            &pages(page, async |c| Ok(client.list_pending(&prefix, c, Some(page.limit)).await?)).await?,
        ),
        Get::Message { id } => one(out, format, &found(client.status(&id).await?, || format!("message {id}"))?),
        Get::Caused { id, page } => {
            let items = pages(page, async |c| {
                found(client.caused_by(&id, c, Some(page.limit)).await?, || format!("message {id}"))
            });
            list(out, format, &items.await?)
        }
        Get::Outcomes { status, prefix, since, page } => {
            let items = pages(page, async |c| Ok(client.outcomes(status, &prefix, since, c, Some(page.limit)).await?));
            list(out, format, &items.await?)
        }
        Get::Turns { node, prefix, page } => {
            let items = pages(page, async |c| Ok(client.turns(node.as_ref(), &prefix, c, Some(page.limit)).await?));
            list(out, format, &items.await?)
        }
        Get::Turn { actor } => {
            one(out, format, &found(client.turn(&actor).await?, || format!("running turn on actor {actor}"))?)
        }
        Get::Nodes => list(out, format, &client.nodes().await?),
        Get::Node { id } => one(out, format, &found(client.node(&id).await?, || format!("node {id}"))?.node),
        Get::Types => list(out, format, &client.types().await?),
        Get::Type { name } => one(out, format, &found(client.get_type(&name).await?, || format!("type {name}"))?),
    }
}

async fn describe_command(
    client: &ActorClient,
    describe: Describe,
    format: Format,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let format = if format == Format::Json { Format::Json } else { Format::Yaml };
    match describe {
        Describe::Actor { actor } => {
            structured(out, format, &found(client.get_actor(&actor).await?, || format!("actor {actor}"))?)
        }
        Describe::Message { id } => {
            structured(out, format, &found(client.status(&id).await?, || format!("message {id}"))?)
        }
        Describe::Node { id } => structured(out, format, &found(client.node(&id).await?, || format!("node {id}"))?),
        Describe::Type { name } => {
            structured(out, format, &found(client.get_type(&name).await?, || format!("type {name}"))?)
        }
    }
}

fn config_command(
    command: ConfigCommand,
    server: Option<String>,
    token: Option<String>,
    env: &Env,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let path = config::path(env)?;
    let mut config = Config::load(&path)?;
    match command {
        ConfigCommand::GetContexts => {
            let rows: Vec<Vec<String>> = config
                .contexts
                .iter()
                .map(|(name, c)| {
                    let current = if config.current_context.as_ref() == Some(name) { "*" } else { "" };
                    vec![current.to_owned(), name.clone(), c.server.clone()]
                })
                .collect();
            write!(out, "{}", output::table(&["CURRENT", "NAME", "SERVER"], &rows))?;
        }
        ConfigCommand::CurrentContext => {
            let name =
                config.current_context.context("no current context; set one with `cicada config use-context`")?;
            writeln!(out, "{name}")?;
        }
        ConfigCommand::UseContext { name } => {
            if !config.contexts.contains_key(&name) {
                bail!("no context '{name}' in {}", path.display());
            }
            config.current_context = Some(name.clone());
            config.save(&path)?;
            writeln!(out, "switched to context {name}")?;
        }
        ConfigCommand::SetContext { name, headers } => {
            let server = server.context("set-context needs --server")?;
            let context = config::Context { server, token, headers: headers.into_iter().collect() };
            config.contexts.insert(name.clone(), context);
            config.current_context.get_or_insert_with(|| name.clone());
            config.save(&path)?;
            writeln!(out, "context {name} set in {}", path.display())?;
        }
    }
    Ok(())
}

/// The items of one page, or of every page with `--all`.
async fn pages<T>(
    page: Page,
    mut fetch: impl AsyncFnMut(Option<&str>) -> anyhow::Result<PageView<T>>,
) -> anyhow::Result<Vec<T>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let next = fetch(cursor.as_deref()).await?;
        items.extend(next.items);
        match next.next {
            Some(next) if page.all => cursor = Some(next),
            _ => return Ok(items),
        }
    }
}

/// `Some` as found; `None` is the error "<what> not found".
fn found<T>(value: Option<T>, what: impl FnOnce() -> String) -> anyhow::Result<T> {
    value.with_context(|| format!("{} not found", what()))
}

/// The result of an action: `text` for a table, `value` for JSON or YAML.
fn done(out: &mut dyn Write, format: Format, text: &str, value: serde_json::Value) -> anyhow::Result<()> {
    match format {
        Format::Table | Format::Wide => writeln!(out, "{text}").map_err(Into::into),
        Format::Json | Format::Yaml => structured(out, format, &value),
    }
}

fn bulk(selection: Selection) -> BulkRequest {
    BulkRequest {
        ids: (!selection.ids.is_empty()).then_some(selection.ids),
        prefix: selection.prefix,
        message_type: selection.message_type,
    }
}
