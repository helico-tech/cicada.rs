//! What the TUI asks the API: the data of a screen, the header summary, and actions.

use anyhow::{anyhow, bail};
use chrono::Utc;
use cicada::api::OutcomeView;
use cicada::store::RemoveTypeResult;
use cicada::time::{Timestamp, format_instant};
use cicada_client::{ActorClient, CancelOutcome};
use serde_json::{Value, json};

use super::app::{Action, Fact, Item, Kind, Overview, ScreenData, Summary, Target};
use super::grid::{GridRow, Rows};
use super::theme::{self, Tone};
use crate::output::{Columns, age, wire};

/// Rows per listing.
const LIMIT: usize = 1000;

/// The rows of `items`, narrow and wide.
pub fn rows<T: Columns>(items: &[T], finished: bool, item: impl Fn(&T) -> Item) -> Rows {
    let now = Utc::now();
    Rows {
        headers: T::headers(false),
        wide_headers: T::headers(true),
        rows: items
            .iter()
            .map(|i| GridRow { item: item(i), cells: i.cells(false, now), wide: i.cells(true, now) })
            .collect(),
        finished,
    }
}

/// The data of the screen `target`.
pub async fn screen(client: &ActorClient, target: &Target) -> anyhow::Result<ScreenData> {
    let limit = Some(LIMIT);
    Ok(match target {
        Target::List { kind, prefix, status } => ScreenData::Grid(match kind {
            Kind::Actors => {
                rows(&client.list_actors(prefix, None, limit).await?.items, false, |a| Item::Actor(a.clone()))
            }
            Kind::Messages => {
                rows(&client.list_pending(prefix, None, limit).await?.items, false, |m| Item::Message(m.id.clone()))
            }
            Kind::Outcomes => {
                // Oldest first from the API; newest first on screen.
                let mut items = client.outcomes(*status, prefix, None, None, limit).await?.items;
                items.reverse();
                rows(&items, true, |o| Item::Message(o.id.clone()))
            }
            Kind::Turns => {
                rows(&client.turns(None, prefix, None, limit).await?.items, false, |t| Item::Turn(t.actor.clone()))
            }
            Kind::Nodes => {
                let nodes: Vec<_> =
                    client.nodes().await?.into_iter().filter(|n| n.id.as_str().starts_with(prefix.as_str())).collect();
                rows(&nodes, false, |n| Item::Node(n.id.clone()))
            }
            Kind::Types => {
                let types: Vec<_> =
                    client.types().await?.into_iter().filter(|t| t.name.starts_with(prefix.as_str())).collect();
                rows(&types, false, |t| Item::Type(t.name.parse().expect("a served type name parses")))
            }
        }),
        Target::Mailbox(actor) => {
            let page = client.mailbox(actor, None, limit).await?.ok_or_else(|| anyhow!("actor {actor} not found"))?;
            ScreenData::Grid(rows(&page.items, false, |m| Item::Message(m.message.id.clone())))
        }
        Target::Actor(actor) => actor_overview(client, actor).await?,
        Target::Message(id) => {
            let (status, caused) = tokio::try_join!(client.status(id), client.caused_by(id, None, limit))?;
            let status = status.ok_or_else(|| anyhow!("message {id} not found"))?;
            let m = &status.message;
            let state = wire(&status.status);
            let mut facts = vec![
                Fact { label: "Status", value: format!("● {state}"), tone: theme::status(&state) },
                fact("Type", &m.message_type),
                fact("To", m.to.to_string()),
                fact("From", m.from.as_ref().map_or("outside an actor".to_owned(), ToString::to_string)),
                fact("Caused by", m.caused_by.as_ref().map_or("-".to_owned(), ToString::to_string)),
                fact("Due", when(m.deliver_at)),
                fact("Attempt", m.attempt.to_string()),
                Fact {
                    label: "Failures",
                    value: m.failures.to_string(),
                    tone: if m.failures > 0 { Tone::Warn } else { Tone::Normal },
                },
            ];
            facts.extend(m.correlation_id.as_ref().map(|c| fact("Correlation", c)));
            facts.extend(status.last_error.as_ref().map(|e| Fact {
                label: "Last error",
                value: e.clone(),
                tone: Tone::Bad,
            }));
            facts.extend(status.error.as_ref().map(|e| Fact { label: "Error", value: e.clone(), tone: Tone::Bad }));
            facts.extend(status.finished_at.map(|t| fact("Finished", when(t))));
            facts.extend(status.node.as_ref().map(|n| Fact { label: "Node", value: n.clone(), tone: Tone::Info }));
            facts.extend(status.lease_until.map(|t| fact("Lease", when(t))));
            if !m.headers.is_empty() {
                let headers: Vec<String> = m.headers.iter().map(|(k, v)| format!("{k}={v}")).collect();
                facts.push(fact("Headers", headers.join(", ")));
            }
            let overview = Overview {
                subject: Item::Message(id.clone()),
                facts,
                panes: vec![("payload".to_owned(), m.payload.clone())],
                pane_follows_selection: false,
                caused_by: m.caused_by.clone(),
                actor: Some(m.to.clone()),
            };
            let caused = caused.map(|p| p.items).unwrap_or_default();
            ScreenData::Overview(overview, vec![rows(&caused, false, |s| Item::Message(s.id.clone()))])
        }
        Target::Node(id) => {
            let detail = client.node(id).await?.ok_or_else(|| anyhow!("node {id} not found"))?;
            let node = &detail.node;
            let health = wire(&node.status);
            let overview = Overview {
                subject: Item::Node(id.clone()),
                facts: vec![
                    Fact { label: "Status", value: format!("● {health}"), tone: theme::status(&health) },
                    fact("Started", when(node.started_at)),
                    fact("Last seen", when(node.last_seen)),
                    fact("Turns", node.running_turns.to_string()),
                    fact("Hosted", format!("{} types", node.hosted.len())),
                ],
                panes: vec![("hosted types".to_owned(), json!(node.hosted))],
                pane_follows_selection: false,
                caused_by: None,
                actor: None,
            };
            ScreenData::Overview(overview, vec![rows(&detail.turns, false, |t| Item::Turn(t.actor.clone()))])
        }
        Target::Type(name) => {
            let view = client.get_type(name).await?.ok_or_else(|| anyhow!("type {name} not found"))?;
            let mut panes: Vec<(String, Value)> = vec![("state".to_owned(), view.state.clone())];
            panes.extend(view.messages.iter().map(|(n, s)| (n.clone(), s.clone())));
            let schema_rows = panes
                .iter()
                .enumerate()
                .map(|(i, (n, schema))| {
                    let kind = if i == 0 { "state" } else { "message" };
                    let fields = schema.get("properties").and_then(Value::as_object).map_or(0, |p| p.len()).to_string();
                    let cells = vec![n.clone(), kind.to_owned(), fields];
                    GridRow { item: Item::Schema(n.clone()), wide: cells.clone(), cells }
                })
                .collect();
            let headers = vec!["SCHEMA", "KIND", "FIELDS"];
            let grid = Rows { headers: headers.clone(), wide_headers: headers, rows: schema_rows, finished: false };
            let overview = Overview {
                subject: Item::Type(name.clone()),
                facts: vec![
                    fact("Name", &view.name),
                    fact("Key", if view.key.is_empty() { "(singleton)".to_owned() } else { view.key.join(":") }),
                    fact("Messages", view.messages.len().to_string()),
                    Fact { label: "Hash", value: short_hash(&view.hash), tone: Tone::Dim },
                ],
                panes,
                pane_follows_selection: true,
                caused_by: None,
                actor: None,
            };
            ScreenData::Overview(overview, vec![grid])
        }
        Target::Describe(item) => ScreenData::Describe(describe(client, item).await?),
    })
}

async fn actor_overview(client: &ActorClient, actor: &cicada::ActorAddress) -> anyhow::Result<ScreenData> {
    let limit = Some(LIMIT);
    let (view, turn, mailbox, outcomes) = tokio::try_join!(
        client.get_actor(actor),
        client.turn(actor),
        client.mailbox(actor, None, limit),
        client.outcomes(None, actor.as_str(), None, None, limit),
    )?;
    let view = view.ok_or_else(|| anyhow!("actor {actor} not found"))?;
    let mailbox = mailbox.map(|p| p.items).unwrap_or_default();
    // The prefix also matches longer addresses; keep this actor's, newest first.
    let outcomes: Vec<OutcomeView> = outcomes.items.into_iter().rev().filter(|o| o.actor == *actor).collect();
    let key = if view.params.is_empty() {
        "(singleton)".to_owned()
    } else {
        view.params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")
    };
    let turn_fact = match &turn {
        Some(t) if t.cancel_requested => {
            Fact { label: "Turn", value: format!("● stopping {} on {}", t.message_id, t.node), tone: Tone::Bad }
        }
        Some(t) => Fact {
            label: "Turn",
            value: format!("● running {} on {}, lease {}", t.message_id, t.node, age(t.lease_until, Utc::now())),
            tone: Tone::Info,
        },
        None => Fact { label: "Turn", value: "idle".to_owned(), tone: Tone::Dim },
    };
    let (mailbox_state, tone) = if view.paused { ("● paused", Tone::Warn) } else { ("● active", Tone::Good) };
    let pending = if mailbox.len() == LIMIT { format!("{LIMIT}+") } else { mailbox.len().to_string() };
    let overview = Overview {
        subject: Item::Actor(actor.clone()),
        facts: vec![
            fact("Type", &view.type_name),
            fact("Key", key),
            fact("Version", view.version.to_string()),
            fact("Created", when(view.created_at)),
            Fact { label: "Mailbox", value: format!("{mailbox_state}, {pending} pending"), tone },
            turn_fact,
        ],
        panes: vec![("state".to_owned(), view.state.clone())],
        pane_follows_selection: false,
        caused_by: None,
        actor: None,
    };
    let tabs = vec![
        rows(&mailbox, false, |m| Item::Message(m.message.id.clone())),
        rows(&outcomes, true, |o| Item::Message(o.id.clone())),
    ];
    Ok(ScreenData::Overview(overview, tabs))
}

async fn describe(client: &ActorClient, item: &Item) -> anyhow::Result<Value> {
    let value = match item {
        Item::Actor(a) => client.get_actor(a).await?.map(serde_json::to_value),
        Item::Message(id) => client.status(id).await?.map(serde_json::to_value),
        Item::Turn(a) => client.turn(a).await?.map(serde_json::to_value),
        Item::Node(id) => client.node(id).await?.map(serde_json::to_value),
        Item::Type(name) => client.get_type(name).await?.map(serde_json::to_value),
        Item::Schema(_) => bail!("a schema is shown on its type's screen"),
    };
    Ok(value.ok_or_else(|| anyhow!("{} not found", super::app::label(item)))??)
}

/// The header summary.
pub async fn summary(client: &ActorClient) -> anyhow::Result<Summary> {
    let (stats, nodes) = tokio::try_join!(client.stats(""), client.nodes())?;
    Ok(Summary { stats, nodes })
}

/// Runs `action`; what it did.
pub async fn act(client: &ActorClient, action: &Action) -> anyhow::Result<String> {
    let found = |done: bool, what: String, text: &str| {
        if done { Ok(format!("{what} {text}")) } else { Err(anyhow!("{what} not found")) }
    };
    match action {
        Action::Delete(Item::Actor(a)) => found(client.delete_actor(a).await?, format!("actor {a}"), "deleted"),
        Action::Delete(Item::Type(name)) => match client.remove_type(name).await? {
            RemoveTypeResult::Removed => Ok(format!("type {name} removed")),
            RemoveTypeResult::Absent => bail!("type {name} not found"),
            RemoveTypeResult::InUse => bail!("type {name} is in use: actors of it exist"),
        },
        Action::Delete(item) => bail!("cannot delete {}", super::app::label(item)),
        Action::StopTurn(a) => found(client.cancel_turn(a).await?, format!("turn on {a}"), "asked to stop"),
        Action::Cancel(id) => match client.cancel(id).await? {
            CancelOutcome::Cancelled => Ok(format!("message {id} cancelled")),
            CancelOutcome::Running => bail!("message {id} is running; stop its turn with ctrl-k"),
            CancelOutcome::AlreadyFinished => bail!("message {id} has already finished"),
            CancelOutcome::NotFound => bail!("message {id} not found"),
        },
        Action::Retry(id) => match client.retry(id).await? {
            Some(receipt) => Ok(format!("message {id} retried as {}", receipt.id)),
            None => bail!("message {id} not found"),
        },
        Action::Pause(a) => found(client.pause_mailbox(a).await?, format!("actor {a}"), "paused"),
        Action::Resume(a) => found(client.resume_mailbox(a).await?, format!("actor {a}"), "resumed"),
        Action::Purge(a) => match client.purge_mailbox(a).await? {
            Some(n) => Ok(format!("{n} messages of actor {a} purged")),
            None => bail!("actor {a} not found"),
        },
        Action::DeliverNow(id) => found(client.deliver_now(id).await?.is_some(), format!("message {id}"), "is due now"),
    }
}

/// `sha256:ece084f9f66e…`; the full hash is in describe.
fn short_hash(hash: &str) -> String {
    match hash.char_indices().nth(19) {
        Some((cut, _)) => format!("{}…", &hash[..cut]),
        None => hash.to_owned(),
    }
}

fn fact(label: &'static str, value: impl Into<String>) -> Fact {
    Fact { label, value: value.into(), tone: Tone::Normal }
}

/// `3m ago · 2026-…` or `in 3m · 2026-…`.
fn when(instant: Timestamp) -> String {
    let short = age(instant, Utc::now());
    let short = if short.starts_with("in ") { short } else { format!("{short} ago") };
    format!("{short} · {}", format_instant(instant))
}
