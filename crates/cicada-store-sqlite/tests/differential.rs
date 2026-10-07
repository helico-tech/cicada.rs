//! Differential test: one scenario covering every `Store` method runs against the reference `InMemoryStore` and
//! against `SqliteStore`, each on its own virtual clock moved in step, and every answer must be the same. Until the
//! conformance suite is wired in, this pins the SQLite store to the reference semantics.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use cicada::ActorAddress;
use cicada::address::UriTemplate;
use cicada::retention::{Keep, Retention};
use cicada::retry::{Backoff, RetryPolicy};
use cicada::store::{
    ActorCreate, ChainEntry, CommitRequest, Delivery, MessageId, NodeId, OutgoingMessage, PageCursor, StateChange,
    Store, StoreResult, TurnOutcome, TypeDef,
};
use cicada::time;
use cicada_store_sqlite::{SqliteOptions, SqliteStore};
use cicada_test::{InMemoryStore, VirtualClock};
use common::TempDb;
use serde_json::json;

const RETENTION: Duration = Duration::from_secs(3600);

fn template() -> UriTemplate {
    UriTemplate::parse("actor://d/{id}").unwrap()
}

fn at(id: &str) -> ActorAddress {
    template().expand(&BTreeMap::from([("id".to_owned(), id.to_owned())])).unwrap()
}

fn mid(id: &str) -> MessageId {
    MessageId::new(id).unwrap()
}

fn msg(id: &str, to: &str) -> OutgoingMessage {
    OutgoingMessage { id: mid(id), ..OutgoingMessage::new(at(to), "M", json!({ "m": id })) }
}

/// Records every answer of the store, debug-formatted.
struct Recorder<'a> {
    store: &'a dyn Store,
    clock: &'a VirtualClock,
    log: Vec<String>,
}

impl Recorder<'_> {
    fn note<T: std::fmt::Debug>(&mut self, step: &str, result: StoreResult<T>) -> Option<T> {
        self.log.push(format!("{step}: {:?}", result.as_ref().map_err(|e| e.to_string())));
        result.ok()
    }

    fn advance(&mut self, by: Duration) {
        self.clock.advance_by(by);
    }

    async fn claim(&mut self, step: &str, ttl: Duration) -> Option<cicada::store::Claim> {
        let types = BTreeSet::from([template()]);
        let claims = self.store.claim_due(&types, &NodeId::new("n").unwrap(), ttl, 1).await;
        self.note(step, claims).and_then(|mut c| c.pop())
    }
}

/// Sorts chain entries by id: their order is unspecified.
fn sorted(mut entries: Vec<ChainEntry>) -> Vec<ChainEntry> {
    entries.sort_by(|a, b| a.id().cmp(b.id()));
    entries
}

async fn scenario(store: &dyn Store, clock: &VirtualClock) -> Vec<String> {
    let mut r = Recorder { store, clock, log: Vec::new() };
    let types = BTreeSet::from([template()]);
    let ttl = Duration::from_secs(30);

    // registry
    let def = TypeDef { template: template(), contract: json!({"c": 1}), contract_hash: "h1".into() };
    r.note("register", store.register(def.clone()).await);
    r.note("register again", store.register(def.clone()).await);
    r.note("register other hash", store.register(TypeDef { contract_hash: "h2".into(), ..def.clone() }).await);
    let overlapping = TypeDef { template: UriTemplate::parse("actor://d/x").unwrap(), ..def.clone() };
    r.note("register overlapping", store.register(overlapping).await);
    r.note("type_def", store.type_def(&template()).await);
    r.note("list_types", store.list_types().await);
    r.note("resolve", store.resolve(&at("a")).await);

    // instances
    for id in ["a", "b", "c", "d"] {
        r.note(&format!("create {id}"), store.create(&at(id), &template(), Some(json!({"id": id}))).await);
    }
    r.note("create a again", store.create(&at("a"), &template(), None).await);
    r.note("get a", store.get(&at("a")).await);
    r.note("get absent", store.get(&at("zz")).await);
    let page = r.note("list page 1", store.list("actor://d/", None, 2).await).unwrap();
    r.note("list page 2", store.list("actor://d/", page.next.as_ref(), 2).await);
    r.note("list prefix", store.list("actor://d/c", None, 10).await);
    r.note("list bad limit", store.list("", None, 0).await);

    // mailbox
    let rich = OutgoingMessage {
        headers: BTreeMap::from([("h".to_owned(), "v".to_owned())]),
        correlation_id: Some("corr".into()),
        retry: Some(RetryPolicy::new(3, Backoff::Fixed { delay: Duration::from_secs(5) }).unwrap()),
        retention: Some(Retention { done: Some(Keep::Remove), failed: Some(Keep::Last { count: 1 }) }),
        from: Some(at("b")),
        ..msg("m1", "a")
    };
    r.note("enqueue m1", store.enqueue(rich.clone()).await);
    r.note("enqueue m1 again", store.enqueue(rich.clone()).await);
    r.note("enqueue absent", store.enqueue(msg("mx", "zz")).await);
    r.note(
        "enqueue m2 later",
        store.enqueue(OutgoingMessage { delivery: Delivery::After(Duration::from_secs(10)), ..msg("m2", "a") }).await,
    );
    r.note(
        "enqueue m3 past",
        store.enqueue(OutgoingMessage { delivery: Delivery::At(time::from_micros(0)), ..msg("m3", "b") }).await,
    );
    let failing_retention = Retention { done: None, failed: Some(Keep::Last { count: 1 }) };
    r.note(
        "enqueue m4",
        store.enqueue(OutgoingMessage { retention: Some(failing_retention.clone()), ..msg("m4", "c") }).await,
    );
    r.note("enqueue m5", store.enqueue(OutgoingMessage { retention: Some(failing_retention), ..msg("m5", "c") }).await);
    r.note(
        "enqueue m6 kept for",
        store
            .enqueue(OutgoingMessage {
                retention: Some(Retention { done: Some(Keep::For { age: Duration::from_secs(7200) }), failed: None }),
                ..msg("m6", "d")
            })
            .await,
    );
    r.note(
        "enqueue too late",
        store.enqueue(OutgoingMessage { delivery: Delivery::At(time::MAX_INSTANT), ..msg("m7", "d") }).await,
    );
    r.note(
        "enqueue beyond max",
        store.enqueue(OutgoingMessage { delivery: Delivery::After(time::MAX_DELAY), ..msg("m8", "d") }).await,
    );
    let page = r.note("pending a 1", store.pending(&at("a"), None, 1).await).unwrap();
    r.note("pending a 2", store.pending(&at("a"), page.next.as_ref(), 1).await);
    r.note("pending absent", store.pending(&at("zz"), None, 10).await);
    r.note("pending bad cursor", store.pending(&at("a"), Some(&PageCursor("x".into())), 10).await);
    let mut cursor = None;
    for n in 0..5 {
        let page =
            r.note(&format!("list_pending {n}"), store.list_pending("actor://d/", cursor.as_ref(), 2).await).unwrap();
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    r.note("next_due_at", store.next_due_at(&types).await);
    r.note("next_due_at none", store.next_due_at(&BTreeSet::new()).await);

    // turns; claims one at a time, a millisecond apart, so the order does not depend on ties
    r.advance(Duration::from_millis(1));
    let c1 = r.claim("claim 1", ttl).await.expect("a claim");
    r.note("status running", store.status(&c1.message.id).await);
    r.note("cancel running", store.cancel(&c1.message.id).await);
    r.note("renew", store.renew(&c1.actor, c1.epoch, Duration::from_secs(60)).await);
    r.note("renew stale", store.renew(&c1.actor, c1.epoch - 1, ttl).await);
    r.note("next_due_at leased", store.next_due_at(&types).await);
    let wrong = CommitRequest::new(c1.actor.clone(), c1.epoch, mid("m2"), TurnOutcome::Done);
    r.note("commit wrong message", store.commit(wrong).await);
    let own_cancel = CommitRequest {
        cancels: vec![c1.message.id.clone()],
        ..CommitRequest::new(c1.actor.clone(), c1.epoch, c1.message.id.clone(), TurnOutcome::Done)
    };
    r.note("commit own cancel", store.commit(own_cancel).await);
    let absent = CommitRequest {
        sends: vec![msg("s0", "zz")],
        ..CommitRequest::new(c1.actor.clone(), c1.epoch, c1.message.id.clone(), TurnOutcome::Done)
    };
    r.note("commit to absent", store.commit(absent).await);
    let reused = CommitRequest {
        sends: vec![msg("m5", "a")],
        ..CommitRequest::new(c1.actor.clone(), c1.epoch, c1.message.id.clone(), TurnOutcome::Done)
    };
    r.note("commit reused id", store.commit(reused).await);
    let full = CommitRequest {
        state: StateChange::NewState(Some(json!({"v": 2}))),
        creates: vec![ActorCreate { address: at("e"), template: template(), state: None }],
        sends: vec![
            OutgoingMessage { caused_by: Some(c1.message.id.clone()), ..msg("s1", "e") },
            OutgoingMessage {
                caused_by: Some(c1.message.id.clone()),
                delivery: Delivery::After(Duration::from_secs(1)),
                ..msg("s2", "a")
            },
        ],
        cancels: vec![mid("m4"), mid("m2"), mid("nope"), mid("m2")],
        ..CommitRequest::new(c1.actor.clone(), c1.epoch, c1.message.id.clone(), TurnOutcome::Done)
    };
    r.note("commit full", store.commit(full).await);
    r.note(
        "commit again",
        store.commit(CommitRequest::new(c1.actor.clone(), c1.epoch, c1.message.id.clone(), TurnOutcome::Done)).await,
    );
    r.note("outcome removed m1", store.outcome(&c1.message.id).await);
    r.note("status removed m1", store.status(&c1.message.id).await);
    r.note("get a after", store.get(&at("a")).await);

    // Every due turn: m3 is rescheduled, m4 and m5 fail (keep-last 1 hides m4), s1 fails, m6 is released once first.
    let mut released = false;
    for n in 2..20 {
        r.advance(Duration::from_millis(1));
        let Some(c) = r.claim(&format!("claim {n}"), ttl).await else { break };
        let outcome = match c.message.id.as_str() {
            "m3" => {
                TurnOutcome::Rescheduled { delivery: Delivery::After(Duration::from_secs(2)), error: "retry".into() }
            }
            "m4" | "m5" | "s1" => TurnOutcome::Failed(format!("failed {}", c.message.id)),
            "m6" if !released => {
                released = true;
                r.note("release", store.release(&c.actor, c.epoch).await);
                r.note("release stale", store.release(&c.actor, c.epoch).await);
                r.note("status released", store.status(&c.message.id).await);
                continue;
            }
            _ => TurnOutcome::Done,
        };
        r.note(
            &format!("commit {n}"),
            store.commit(CommitRequest::new(c.actor.clone(), c.epoch, c.message.id.clone(), outcome)).await,
        );
    }
    r.note("outcome m4 trimmed", store.outcome(&mid("m4")).await);
    r.note("pending b", store.pending(&at("b"), None, 10).await);
    r.advance(Duration::from_secs(3));
    let stale = r.claim("claim for a stale lease", Duration::from_secs(1)).await.expect("a claim");
    r.advance(Duration::from_secs(2));
    r.note("status expired lease", store.status(&stale.message.id).await);
    r.note("cancel expired lease", store.cancel(&stale.message.id).await);
    r.note(
        "commit voided lease",
        store
            .commit(CommitRequest::new(stale.actor.clone(), stale.epoch, stale.message.id.clone(), TurnOutcome::Done))
            .await,
    );
    r.note("cancel finished", store.cancel(&stale.message.id).await);
    r.note("cancel unknown", store.cancel(&mid("nope")).await);

    // the remaining due turns
    for n in 0..4 {
        r.advance(Duration::from_millis(1));
        let Some(c) = r.claim(&format!("drain {n}"), ttl).await else { break };
        r.note(
            &format!("commit drain {n}"),
            store.commit(CommitRequest::new(c.actor.clone(), c.epoch, c.message.id.clone(), TurnOutcome::Done)).await,
        );
    }

    // a stop purges the rest of the mailbox and removes the instance
    r.note("enqueue st1", store.enqueue(msg("st1", "c")).await);
    r.note(
        "enqueue st2",
        store.enqueue(OutgoingMessage { delivery: Delivery::After(Duration::from_secs(60)), ..msg("st2", "c") }).await,
    );
    r.advance(Duration::from_millis(1));
    let c = r.claim("claim st1", ttl).await.expect("a claim");
    let stop = CommitRequest {
        state: StateChange::Stop,
        ..CommitRequest::new(c.actor.clone(), c.epoch, c.message.id.clone(), TurnOutcome::Done)
    };
    r.note("commit stop", store.commit(stop).await);
    r.note("get c stopped", store.get(&at("c")).await);
    r.note("outcome st2 purged", store.outcome(&mid("st2")).await);
    r.note("create c again", store.create(&at("c"), &template(), None).await);
    r.note("get c recreated", store.get(&at("c")).await);

    // listings of outcomes and chains
    let epoch_start = time::from_micros(0);
    let page = r.note("failures 1", store.failures("actor://d/", epoch_start, None, 1).await).unwrap();
    r.note("failures 2", store.failures("actor://d/", epoch_start, page.next.as_ref(), 1).await);
    r.note("failures prefix", store.failures("actor://d/e", epoch_start, None, 10).await);
    let ids: Vec<MessageId> =
        ["m1", "m2", "m3", "m4", "m5", "m6", "s1", "s2", "nope", "m1"].into_iter().map(mid).collect();
    r.note("chain_entries", store.chain_entries(&ids).await.map(sorted));
    r.note("chain_children", store.chain_children(&[mid("m1")], 10).await.map(sorted));
    r.note("chain_children none", store.chain_children(&[], 10).await);

    // delete purges the mailbox
    r.note(
        "enqueue to b",
        store.enqueue(OutgoingMessage { delivery: Delivery::After(Duration::from_secs(60)), ..msg("p1", "b") }).await,
    );
    r.note("delete b", store.delete(&at("b")).await);
    r.note("delete b again", store.delete(&at("b")).await);
    r.note("outcome purged", store.outcome(&mid("p1")).await);

    // retention: For keeps m6 visible longer; the rest expire with the store retention
    r.note("sweep now", store.sweep_expired().await);
    r.advance(RETENTION + Duration::from_secs(1));
    r.note("outcome m5 after retention", store.outcome(&mid("m5")).await);
    r.note("outcome m6 kept", store.outcome(&mid("m6")).await);
    r.note("enqueue reused expired id", store.enqueue(msg("s2", "a")).await);
    r.note("sweep after retention", store.sweep_expired().await);
    r.note("chain after sweep", store.chain_entries(&ids).await.map(sorted));
    r.advance(Duration::from_secs(7200));
    r.note("sweep after kept", store.sweep_expired().await);
    r.note("outcome m6 gone", store.outcome(&mid("m6")).await);
    r.note("now", store.now().await);
    r.log
}

#[tokio::test]
async fn sqlite_answers_like_the_in_memory_reference() {
    let start = cicada_test::default_start();
    let memory_clock = Arc::new(VirtualClock::new(start));
    let memory = InMemoryStore::new(memory_clock.clone(), RETENTION);
    let expected = scenario(&memory, &memory_clock).await;

    let db = TempDb::new();
    let sqlite_clock = Arc::new(VirtualClock::new(start));
    let options = SqliteOptions { clock: sqlite_clock.clone(), retention: RETENTION, ..SqliteOptions::default() };
    let sqlite = SqliteStore::open(&db.file, options).await.unwrap();
    let actual = scenario(&sqlite, &sqlite_clock).await;
    sqlite.close().await;

    if std::env::var_os("CICADA_DIFF_LOG").is_some() {
        actual.iter().for_each(|line| eprintln!("{line}"));
    }
    for (i, (e, a)) in expected.iter().zip(&actual).enumerate() {
        assert_eq!(a, e, "step {i} differs");
    }
    assert_eq!(actual.len(), expected.len());
}
