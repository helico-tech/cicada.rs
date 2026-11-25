//! A column added under an open store (ADR 0025): a store runs the operations of a node, another connection adds a
//! column with a default to every table of the schema, and the store runs them again. A statement that reads with `*`
//! and that a connection has prepared fails then on PostgreSQL and crashes the worker of sqlx on SQLite; each SQL store
//! runs this in its own tests. A source test finds no `*` in a reading statement of either store, also in one that the
//! operations miss.

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use crate::address::{ActorAddress, TypeName};
use crate::store::{
    ActorCreate, Backend, CancelResult, CommitRequest, Control, ControlEffect, CreateResult, DeleteResult,
    DeliverNowResult, Delivery, MessageId, MessageStatus, NodeId, OutcomeStatus, OutgoingMessage, Page, PageCursor,
    RemoveTypeResult, StateChange, TurnOutcome,
};
use crate::tests::conformance::{committed, enqueue_ok, type_def};
use crate::watch::{WatchChange, WatchTarget};
use serde_json::json;

/// The column every table gets: one with a default, as a migration that may be additive adds it.
pub const ADDED_COLUMN: &str = "added_later integer NOT NULL DEFAULT 7";

/// Every item of a listing, one page of one item after the other, so that every listing also runs with a cursor.
async fn pages<T, Fut>(mut page: impl FnMut(Option<PageCursor>) -> Fut) -> Vec<T>
where
    Fut: Future<Output = Page<T>>,
{
    let (mut items, mut cursor) = (Vec::new(), None);
    loop {
        let next = page(cursor).await;
        items.extend(next.items);
        match next.next {
            Some(c) => cursor = Some(c),
            None => return items,
        }
    }
}

/// Runs the operations a node runs on `store` once, every listing page by page, checking each answer; `round` keeps
/// two rounds apart.
pub async fn operations(store: &dyn Backend, round: &str) {
    let name = TypeName::parse("added").unwrap();
    let spare = TypeName::parse(&format!("spare-{round}")).unwrap();
    let types = BTreeSet::from([name.clone()]);
    let node = NodeId::new(format!("node-{round}")).unwrap();
    let at = |key: &str| ActorAddress::of(&name, &format!("{round}-{key}")).unwrap();
    let (actor, other, watcher, child) = (at("actor"), at("other"), at("watcher"), at("child"));
    let prefix = &format!("added/{round}-");
    // References, which the closures that page a listing copy into their futures.
    let (actor_ref, watcher_ref) = (&actor, &watcher);
    let ttl = Duration::from_secs(3600);
    let since = chrono::DateTime::UNIX_EPOCH;

    // The registry, the actors and the watches.
    store.register(type_def(&name, "added")).await.unwrap();
    store.register(type_def(&spare, "spare")).await.unwrap();
    assert!(store.list_types().await.unwrap().iter().any(|t| t.name == spare));
    assert_eq!(store.remove_type(&spare).await.unwrap(), RemoveTypeResult::Removed);
    for address in [&actor, &other, &watcher] {
        assert_eq!(store.create(address, json!({"n": 0}), None).await.unwrap(), CreateResult::Created);
    }
    assert_eq!(store.get(&actor).await.unwrap().expect("created").state, json!({"n": 0}));
    assert_eq!(pages(|c| async move { store.list(prefix, c.as_ref(), 1).await.unwrap() }).await.len(), 3);
    store.watch(&WatchTarget::from(actor.clone()), &watcher).await.unwrap();
    store.watch(&WatchTarget::parse("added").unwrap(), &watcher).await.unwrap();
    let watches = pages(|c| async move { store.watches(Some(watcher_ref), None, c.as_ref(), 1).await.unwrap() }).await;
    assert_eq!(watches.len(), 2);

    // Messages, listed page by page.
    let send = |to: &ActorAddress, n: i64| OutgoingMessage::new(to.clone(), "Msg", json!(n));
    let later = |n: i64| OutgoingMessage { delivery: Delivery::After(ttl), ..send(&actor, n) };
    let first = enqueue_ok(store, send(&actor, 1)).await;
    let second = enqueue_ok(store, send(&actor, 2)).await;
    let third = enqueue_ok(store, send(&actor, 3)).await;
    let (to_cancel, cancelled_in_commit, to_deliver) =
        (enqueue_ok(store, later(4)).await, enqueue_ok(store, later(5)).await, enqueue_ok(store, later(6)).await);
    enqueue_ok(store, send(&other, 7)).await;
    let mailbox = pages(|c| async move { store.pending(actor_ref, c.as_ref(), 1).await.unwrap() }).await;
    assert_eq!(mailbox.len(), 6);
    let all = pages(|c| async move { store.list_pending(prefix, c.as_ref(), 1).await.unwrap() }).await;
    assert_eq!(all.len(), 7);
    assert!(matches!(store.status(&first.id).await.unwrap(), Some(MessageStatus::Pending(_))));
    assert!(store.next_due_at(&types, &crate::tests::owner()).await.unwrap().is_some());

    // A turn that commits state, a send, a child, a cancel and a purge of another actor.
    let claims = store.claim_due(&types, &node, ttl, 10).await.unwrap();
    assert_eq!(claims.len(), 2, "the actor and the other");
    let claim = claims.iter().find(|c| c.actor == actor).expect("the actor is claimed");
    assert_eq!(claim.message.id, first.id);
    store.renew(&actor, claim.epoch, ttl).await.unwrap();
    let node_ref = &node;
    let running = pages(|c| async move { store.running_turns(Some(node_ref), "", c.as_ref(), 1).await.unwrap() }).await;
    assert_eq!(running.len(), 2);
    assert!(store.running_turn(&actor).await.unwrap().is_some());
    assert!(matches!(store.status(&first.id).await.unwrap(), Some(MessageStatus::Running { .. })));
    let other_claim = claims.iter().find(|c| c.actor == other).expect("the other is claimed");
    let mut commit = CommitRequest::new(actor.clone(), claim.epoch, first.id.clone(), TurnOutcome::Done);
    commit.state = StateChange::NewState(json!({"n": 1}));
    commit.creates = vec![ActorCreate { address: child.clone(), state: json!({}), child: true, pinned_to: None }];
    let caused_by_first = |message: OutgoingMessage| OutgoingMessage { caused_by: Some(first.id.clone()), ..message };
    commit.sends = vec![caused_by_first(send(&watcher, 8)), caused_by_first(send(&child, 9))];
    commit.cancels = vec![cancelled_in_commit.id.clone()];
    commit.watches = vec![WatchChange { target: WatchTarget::from(other.clone()), set: true, existing: false }];
    commit.controls = vec![Control { effect: ControlEffect::Purge, actor: other.clone() }];
    let done = committed(store.commit(commit).await.unwrap());
    assert_eq!((done.sent.len(), done.cancelled.as_slice()), (2, std::slice::from_ref(&cancelled_in_commit.id)));
    assert!(matches!(store.status(&first.id).await.unwrap(), Some(MessageStatus::Finished(_))));
    let children = pages(|c| async move { store.children(actor_ref, c.as_ref(), 1).await.unwrap() }).await;
    assert_eq!(children, std::slice::from_ref(&child));
    let caused = pages(|c| {
        let id = first.id.clone();
        async move { store.caused_by(&id, c.as_ref(), 1).await.unwrap() }
    })
    .await;
    assert_eq!(caused.len(), 2);
    let other_done =
        CommitRequest::new(other.clone(), other_claim.epoch, other_claim.message.id.clone(), TurnOutcome::Done);
    committed(store.commit(other_done).await.unwrap());

    // A retry, then a failure that tells the watcher.
    let claim = store.claim_due(&types, &node, ttl, 10).await.unwrap();
    let claim = claim.iter().find(|c| c.actor == actor).expect("the actor is claimed again");
    assert_eq!(claim.message.id, second.id);
    let retry = TurnOutcome::Rescheduled { delivery: Delivery::After(ttl), error: "again".into() };
    committed(store.commit(CommitRequest::new(actor.clone(), claim.epoch, second.id.clone(), retry)).await.unwrap());
    let claim = store.claim_due(&types, &node, ttl, 10).await.unwrap();
    let claim = claim.iter().find(|c| c.actor == actor).expect("the actor is claimed a third time");
    assert_eq!(claim.message.id, third.id);
    let failed = CommitRequest::new(actor.clone(), claim.epoch, third.id.clone(), TurnOutcome::Failed("x".into()));
    committed(store.commit(failed).await.unwrap());
    let failures =
        pages(
            |c| async move { store.outcomes(Some(OutcomeStatus::Failed), prefix, since, c.as_ref(), 1).await.unwrap() },
        )
        .await;
    assert_eq!(failures.iter().map(|o| &o.message_id).collect::<Vec<_>>(), [&third.id]);
    let outcomes = pages(|c| async move { store.outcomes(None, prefix, since, c.as_ref(), 1).await.unwrap() }).await;
    assert_eq!(outcomes.len(), 4, "two done, one cancelled and one failed: {outcomes:?}");
    let told = pages(|c| async move { store.pending(watcher_ref, c.as_ref(), 1).await.unwrap() }).await;
    assert!(told.iter().any(|m| m.notification), "a Failed");

    // The operator's calls, and the end of the actor and its child.
    assert!(matches!(store.cancel(&to_cancel.id).await.unwrap(), CancelResult::Cancelled(_)));
    assert!(matches!(store.deliver_now(&to_deliver.id).await.unwrap(), DeliverNowResult::Delivered(_)));
    assert!(store.set_paused(&actor, true).await.unwrap());
    assert!(store.set_paused(&actor, false).await.unwrap());
    assert_eq!(store.purge(&actor).await.unwrap(), Some(2), "the retry and the message made due");
    assert_eq!(store.stats(prefix).await.unwrap().actors, 4);
    store.heartbeat_node(&node, &types, &Default::default()).await.unwrap();
    assert!(store.list_nodes().await.unwrap().iter().any(|n| n.id == node));
    store.remove_node(&node).await.unwrap();
    assert!(store.request_cancel(&actor).await.is_ok());
    assert!(matches!(store.delete(&actor, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. }));
    assert!(store.get(&child).await.unwrap().is_none(), "the child went with its parent");
    assert!(store.unwatch(&WatchTarget::from(actor.clone()), &watcher).await.is_ok());
    // The next round claims only its own actors.
    for address in [&other, &watcher] {
        assert!(matches!(store.delete(address, &MessageId::random()).await.unwrap(), DeleteResult::Deleted { .. }));
    }
    store.sweep_expired().await.unwrap();
}

/// The sources of both SQL stores, without their tests.
const STORE_SOURCES: [(&str, &str); 8] = [
    ("sqlite/store.rs", include_str!("../store/sqlite/store.rs")),
    ("sqlite/schema.rs", include_str!("../store/sqlite/schema.rs")),
    ("sqlite/conn.rs", include_str!("../store/sqlite/conn.rs")),
    ("sqlite/rows.rs", include_str!("../store/sqlite/rows.rs")),
    ("postgres/store.rs", include_str!("../store/postgres/store.rs")),
    ("postgres/schema.rs", include_str!("../store/postgres/schema.rs")),
    ("postgres/listener.rs", include_str!("../store/postgres/listener.rs")),
    ("postgres/config.rs", include_str!("../store/postgres/config.rs")),
];

/// The texts of the statements in `source`, comment lines left out: each string literal outside a `concat!`, and the
/// literals of each `concat!` joined into one text, with a space for each other part of it (such as
/// `message_columns!()`), so that a statement built from pieces is read as a whole.
fn statement_texts(source: &str) -> Vec<String> {
    let source: String =
        source.lines().filter(|l| !l.trim_start().starts_with("//")).map(|l| format!("{l}\n")).collect();
    let chars: Vec<char> = source.chars().collect();
    let (mut texts, mut group, mut depth, mut i) = (Vec::new(), None::<(String, usize)>, 0usize, 0);
    while i < chars.len() {
        let rest: String = chars[i..chars.len().min(i + 8)].iter().collect();
        if rest.starts_with("concat!(") && group.is_none() {
            depth += 1;
            group = Some((String::new(), depth));
            i += 8;
            continue;
        }
        let raw = rest.starts_with('r') && rest[1..].trim_start_matches('#').starts_with('"');
        if chars[i] == '"' || raw {
            // A string literal: plain with escapes, or raw with as many `#` as it opens with.
            let hashes = if raw { chars[i + 1..].iter().take_while(|c| **c == '#').count() } else { 0 };
            let mut j = i + if raw { 2 + hashes } else { 1 };
            let mut literal = String::new();
            loop {
                if !raw && chars[j] == '\\' {
                    literal.push(chars[j + 1]);
                    j += 2;
                } else if chars[j] == '"' && chars[j + 1..].iter().take(hashes).all(|c| *c == '#') {
                    break;
                } else {
                    literal.push(chars[j]);
                    j += 1;
                }
            }
            match &mut group {
                Some((text, _)) => text.push_str(&literal),
                None => texts.push(literal),
            }
            i = j + 1 + hashes;
            continue;
        }
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                if group.as_ref().is_some_and(|(_, opened)| *opened == depth) {
                    texts.push(group.take().expect("open").0);
                }
                depth = depth.saturating_sub(1);
            }
            c if !c.is_whitespace() && c != ',' => {
                if let Some((text, _)) = &mut group {
                    text.push(' ');
                }
            }
            _ => {}
        }
        i += 1;
    }
    texts
}

/// The places in `source` where a statement reads with `*`: in the select list of a `SELECT` (up to its `FROM` or the
/// end of the statement, `count(*)` aside) or after `RETURNING`.
fn star_reads(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for text in statement_texts(source) {
        for keyword in ["SELECT", "RETURNING"] {
            for (start, _) in text.match_indices(keyword) {
                let rest = &text[start..];
                let list = &rest[..rest.find("FROM").unwrap_or(rest.len())];
                if list.replace("count(*)", "").contains('*') {
                    found.push(list.split_whitespace().collect::<Vec<_>>().join(" "));
                }
            }
        }
    }
    found
}

/// Scenario: A column is added under an open store (every statement): no reading statement of either SQL store reads
/// with `*`, so none of them breaks when a column is added.
#[test]
fn no_statement_of_a_sql_store_reads_with_a_star() {
    for (file, source) in STORE_SOURCES {
        assert_eq!(star_reads(source), Vec::<String>::new(), "{file}");
    }
    // The scan reads the statements: those of the stores, each `concat!` as one text.
    for (file, source) in [STORE_SOURCES[0], STORE_SOURCES[4]] {
        let texts = statement_texts(source);
        let reads = texts.iter().filter(|t| t.contains("SELECT") && t.contains("FROM messages")).count();
        assert!(reads >= 10, "{file}: {reads} reads of messages");
        assert!(texts.iter().any(|t| t.starts_with("SELECT") && t.contains("  FROM messages WHERE id = ")), "{file}");
    }
    let samples = [
        r#"query("SELECT * FROM messages WHERE id = ?")"#,
        r#"query("SELECT m.*, a.lease_owner FROM messages m")"#,
        "concat!(\"SELECT *\",\n \" FROM outcomes\")",
        r#"query(concat!("SELECT ", "*", " FROM messages WHERE id = $1"))"#,
        r##"query(concat!(r#"SELECT "#, message_columns!("m."), ", *", visible!(2, 3), " FROM messages m"))"##,
        r#"query("UPDATE actors SET paused = $1 RETURNING *")"#,
    ];
    for sample in samples {
        assert_eq!(star_reads(sample).len(), 1, "{sample}");
    }
    assert!(star_reads(r#"query("SELECT count(*) AS n FROM actors").execute(&mut *tx.c)"#).is_empty());
    assert!(star_reads(r#"concat!("SELECT ", message_columns!(), " FROM messages")"#).is_empty());
    assert!(star_reads(r#"query("SELECT pg_notify($1, $2)").execute(&mut *tx.c); let w = "*";"#).is_empty());
    assert!(star_reads("/// a statement (`SELECT … FOR UPDATE`) reads `*`\n").is_empty(), "a comment");
}
