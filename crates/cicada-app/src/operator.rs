//! The operator commands: one parsed [`Cli`] run against a node over HTTP, through [`ActorClient`].

use std::io::Write;
use std::time::Duration;

use anyhow::{Context as _, bail};
use cicada::api::{BulkRequest, PageView};
use cicada::store::RemoveTypeResult;
use cicada::time::format_instant;
use cicada::{CreateOptions, SendOptions};
use cicada_client::{ActorClient, CancelOutcome};
use serde_json::json;

use crate::Env;
use crate::cli::{
    Cancel, Cli, Command, ConfigCommand, Create, Delete, Describe, Get, Page, Retry, Selection, WatchArgs,
};
use crate::config::{self, Config, ContextEntry, ContextFile, Overrides};
use crate::output::{self, Format, list, one, structured};

/// `error` and its causes, each cause only when the text so far does not already hold it (a `ClientError` repeats
/// its source in its own message).
pub(crate) fn error_text(error: &anyhow::Error) -> String {
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

/// Runs `cli` with the environment `env`, writing results to `out`; `name` is the binary as it was called, for the
/// hints. Any error means exit code 1.
pub(crate) async fn run(cli: Cli, name: &str, env: &Env, out: &mut (dyn Write + Send)) -> anyhow::Result<()> {
    if let Command::Config(command) = cli.command {
        return config_command(command, cli.server, cli.token, name, env, out);
    }
    let flags = Overrides { context: cli.context, server: cli.server, token: cli.token };
    let target = config::resolve(&flags, env, &Config::load(&config::path(env)?)?, name)?;
    #[cfg(feature = "tui")]
    if let Command::Tui = cli.command {
        return crate::tui::run(target).await;
    }
    let client = target.client(TIMEOUT)?;
    let format = cli.output.unwrap_or(Format::Table);
    match cli.command {
        Command::Get(get) => get_command(&client, get, format, out).await,
        Command::Describe(describe) => {
            describe_command(&client, describe, cli.output.unwrap_or(Format::Yaml), out).await
        }
        Command::Create(Create::Actor { actor, state, pinned_to }) => {
            let created = client.create_actor_with(&actor, state, CreateOptions { pinned_to }).await?;
            let verb = if created.created { "created" } else { "exists, left untouched" };
            done(out, format, &format!("actor {} {verb}", created.address), json!(created))
        }
        Command::Create(Create::Watch(WatchArgs { target, watcher })) => {
            let created = client.watch(&target, &watcher).await?;
            let verb = if created { "set" } else { "exists, left untouched" };
            let text = format!("watch of {watcher} on {target} {verb}");
            done(out, format, &text, json!({"target": target, "watcher": watcher, "created": created}))
        }
        Command::Delete(Delete::Actor { actor }) => {
            found(client.delete_actor(&actor).await?.then_some(()), || format!("actor {actor}"))?;
            writeln!(out, "actor {actor} deleted").map_err(Into::into)
        }
        Command::Delete(Delete::Watch(WatchArgs { target, watcher })) => {
            found(client.unwatch(&target, &watcher).await?.then_some(()), || {
                format!("watch of {watcher} on {target}")
            })?;
            writeln!(out, "watch of {watcher} on {target} removed").map_err(Into::into)
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
            CancelOutcome::Running => bail!("message {id} is running; stop its turn with `{name} cancel turn`"),
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
        Command::Config(_) => unreachable!("handled above"),
        #[cfg(feature = "tui")]
        Command::Tui => unreachable!("handled above"),
    }
}

// The closures given to `pages` copy references into `async move` blocks: the future of a command must be `Send`, which
// the compiler cannot prove for an async closure that borrows.
async fn get_command(
    client: &ActorClient,
    get: Get,
    format: Format,
    out: &mut (dyn Write + Send),
) -> anyhow::Result<()> {
    match get {
        Get::Actors { prefix, page } => {
            let prefix = &prefix;
            let items =
                pages(page, |c| async move { Ok(client.list_actors(prefix, c.as_deref(), Some(page.limit)).await?) });
            list(out, format, &items.await?)
        }
        Get::Actor { actor } => one(out, format, &found(client.get_actor(&actor).await?, || format!("actor {actor}"))?),
        Get::Children { actor, page } => {
            let actor = &actor;
            let items = pages(page, |c| async move {
                found(client.children(actor, c.as_deref(), Some(page.limit)).await?, || format!("actor {actor}"))
            });
            list(out, format, &items.await?)
        }
        Get::Mailbox { actor, page } => {
            let actor = &actor;
            let items = pages(page, |c| async move {
                found(client.mailbox(actor, c.as_deref(), Some(page.limit)).await?, || format!("actor {actor}"))
            });
            list(out, format, &items.await?)
        }
        Get::Messages { prefix, page } => {
            let prefix = &prefix;
            let items =
                pages(page, |c| async move { Ok(client.list_pending(prefix, c.as_deref(), Some(page.limit)).await?) });
            list(out, format, &items.await?)
        }
        Get::Message { id } => one(out, format, &found(client.status(&id).await?, || format!("message {id}"))?),
        Get::Caused { id, page } => {
            let id = &id;
            let items = pages(page, |c| async move {
                found(client.caused_by(id, c.as_deref(), Some(page.limit)).await?, || format!("message {id}"))
            });
            list(out, format, &items.await?)
        }
        Get::Outcomes { status, prefix, since, page } => {
            let prefix = &prefix;
            let items = pages(page, |c| async move {
                Ok(client.outcomes(status, prefix, since, c.as_deref(), Some(page.limit)).await?)
            });
            list(out, format, &items.await?)
        }
        Get::Turns { node, prefix, page } => {
            let (node, prefix) = (node.as_ref(), &prefix);
            let items =
                pages(page, |c| async move { Ok(client.turns(node, prefix, c.as_deref(), Some(page.limit)).await?) });
            list(out, format, &items.await?)
        }
        Get::Turn { actor } => {
            one(out, format, &found(client.turn(&actor).await?, || format!("running turn on actor {actor}"))?)
        }
        Get::Nodes => list(out, format, &client.nodes().await?),
        Get::Node { id } => one(out, format, &found(client.node(&id).await?, || format!("node {id}"))?),
        Get::Types => list(out, format, &client.types().await?),
        Get::Watches { watcher, target, page } => {
            let (watcher, target) = (watcher.as_ref(), target.as_ref());
            let items =
                pages(
                    page,
                    |c| async move { Ok(client.watches(watcher, target, c.as_deref(), Some(page.limit)).await?) },
                );
            list(out, format, &items.await?)
        }
        Get::Type { name } => one(out, format, &found(client.get_type(&name).await?, || format!("type {name}"))?),
    }
}

async fn describe_command(
    client: &ActorClient,
    describe: Describe,
    format: Format,
    out: &mut (dyn Write + Send),
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
    name: &str,
    env: &Env,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    match command {
        ConfigCommand::GetContexts => {
            let file = ContextFile::load(env)?;
            let rows: Vec<Vec<String>> = file
                .names()
                .map(|name| {
                    let current = if file.current() == Some(name) { "*" } else { "" };
                    let server = file.context(name).map(|c| c.server).unwrap_or_default();
                    vec![current.to_owned(), name.to_owned(), server]
                })
                .collect();
            write!(out, "{}", output::table(&["CURRENT", "NAME", "SERVER"], &rows))?;
        }
        ConfigCommand::CurrentContext => {
            let file = ContextFile::load(env)?;
            let current = file
                .current()
                .with_context(|| format!("no current context; set one with `{name} config use-context`"))?;
            writeln!(out, "{current}")?;
        }
        ConfigCommand::UseContext { name } => {
            ContextFile::update(env, |file| file.use_context(&name))?;
            writeln!(out, "switched to context {name}")?;
        }
        ConfigCommand::SetContext { name, headers } => {
            let path = ContextFile::update(env, |file| {
                let server = server.context("set-context needs --server")?;
                file.set_context(&name, ContextEntry { server, token, headers: headers.into_iter().collect() });
                Ok(file.path().to_owned())
            })?;
            writeln!(out, "context {name} set in {}", path.display())?;
        }
    }
    Ok(())
}

/// The items of one page, or of every page with `--all`; `fetch` gets the cursor of the page.
async fn pages<T, F, Fut>(page: Page, mut fetch: F) -> anyhow::Result<Vec<T>>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = anyhow::Result<PageView<T>>>,
{
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let next = fetch(cursor.take()).await?;
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
fn done(out: &mut (dyn Write + Send), format: Format, text: &str, value: serde_json::Value) -> anyhow::Result<()> {
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
