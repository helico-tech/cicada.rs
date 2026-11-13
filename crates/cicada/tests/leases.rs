//! An executor outside the system leases, renews and releases turns through the public core API
//! (`ActorSystem::lease_turns`, `renew_turn`, `release_turn`; ADR 0037): on a system without handlers, with the server's
//! TTL, hash checks before any claim, lease exclusivity shared with local nodes, epoch fencing and the server's life
//! cycle. Public API only, on virtual time.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::prelude::*;
use cicada::runtime::{
    LeaseTurnsResult, MAX_LEASE_BATCH, MAX_LEASE_TYPES, NodeConfig, NodeState, RenewTurnResult, TurnLease,
};
use cicada::store::{Message, MessageStatus, NodeId, StoreError};
use cicada::testing::{TokioClock, default_start};
use cicada::time::Timestamp;
use cicada::{OpError, SendResult, Store, TypeName};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::advance;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Counter {
    id: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CounterMsg {
    Add { n: i64 },
    Fail,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct CounterState {
    total: i64,
}

struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        match message {
            CounterMsg::Add { n } => turn.state_mut().total += n,
            CounterMsg::Fail => return Err(HandlerError::msg("failed on purpose")),
        }
        Ok(())
    }
}

fn counter(id: &str) -> Counter {
    Counter { id: id.into() }
}

fn address(id: &str) -> ActorAddress {
    counter(id).address().unwrap()
}

fn name(n: &str) -> TypeName {
    TypeName::parse(n).unwrap()
}

fn worker(id: &str) -> NodeId {
    NodeId::new(id).unwrap()
}

/// A store on tokio's paused clock.
fn store() -> Store {
    Store::memory_on(Arc::new(TokioClock::new(default_start())))
}

fn config(ttl: Duration) -> NodeConfig {
    NodeConfig { lease_ttl: ttl, ..NodeConfig::default() }
}

/// A system that hosts nothing, with a lease TTL of `ttl`.
fn server(store: &Store, ttl: Duration) -> ActorSystem {
    ActorSystem::builder(store.clone()).config(config(ttl)).build()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn plus(instant: Timestamp, duration: Duration) -> Timestamp {
    instant + chrono::Duration::from_std(duration).unwrap()
}

/// The counter type with the hash it was built with.
fn selection() -> BTreeMap<TypeName, String> {
    BTreeMap::from([(name("counter"), contract_of::<CounterActor>().unwrap().hash().to_owned())])
}

/// Registers the counter's contract and creates the counters `ids`.
async fn setup(system: &ActorSystem, ids: &[&str]) {
    system.register_contract(&contract_of::<CounterActor>().unwrap()).await.unwrap();
    for id in ids {
        system.create(&counter(id), &CounterState::default()).await.unwrap();
    }
}

async fn send(system: &ActorSystem, id: &str, message: CounterMsg) -> Message {
    send_with(system, id, message, SendOptions::default()).await
}

async fn send_with(system: &ActorSystem, id: &str, message: CounterMsg, options: SendOptions) -> Message {
    match system.send_with(&counter(id), message, options).await.unwrap() {
        SendResult::Accepted(m) => m,
        other => panic!("expected Accepted, got {other:?}"),
    }
}

fn leased(result: LeaseTurnsResult) -> Vec<TurnLease> {
    match result {
        LeaseTurnsResult::Leased(leases) => leases,
        other => panic!("expected Leased, got {other:?}"),
    }
}

fn one(result: LeaseTurnsResult) -> TurnLease {
    let mut leases = leased(result);
    assert_eq!(leases.len(), 1, "{leases:?}");
    leases.remove(0)
}

async fn lease(system: &ActorSystem, owner: &str) -> Vec<TurnLease> {
    leased(system.lease_turns(&worker(owner), &selection(), 10).await.unwrap())
}

/// The lease expiry, TTL and cancel flag of a renewal.
fn renewed(result: RenewTurnResult) -> (Timestamp, Duration, bool) {
    match result {
        RenewTurnResult::Renewed { lease_until, ttl, cancel_requested } => (lease_until, ttl, cancel_requested),
        RenewTurnResult::LeaseLost => panic!("expected Renewed, got LeaseLost"),
    }
}

async fn pending_head(system: &ActorSystem, id: &str) -> Message {
    system.pending(&address(id), None, 10).await.unwrap().expect("the counter").items.remove(0)
}

fn assert_invalid<T: std::fmt::Debug>(result: Result<T, OpError>) {
    assert!(matches!(result, Err(OpError::Invalid(_))), "expected Invalid, got {result:?}");
}

fn assert_unavailable<T: std::fmt::Debug>(result: Result<T, OpError>) {
    assert!(matches!(result, Err(OpError::Store(StoreError::Unavailable(_)))), "expected Unavailable, got {result:?}");
}

/// Scenario: A server without handlers. Scenario: All lease fields.
#[tokio::test(start_paused = true)]
async fn an_unstarted_system_without_handlers_leases_a_turn_with_every_field() {
    let store = store();
    let server = server(&store, secs(30));
    setup(&server, &["c1"]).await;
    let options = SendOptions {
        correlation_id: Some("order-7".into()),
        headers: BTreeMap::from([("tenant".to_owned(), "acme".to_owned())]),
        retry: Some(RetryPolicy::new(3, Backoff::fixed(secs(1)).unwrap()).unwrap()),
        retention: Some(Retention::REMOVE_ON_FAILED),
        ..SendOptions::default()
    };
    let sent = send_with(&server, "c1", CounterMsg::Fail, options).await;
    // A local host gives the message a failure: it fails once and is rescheduled.
    let host = ActorSystem::builder(store.clone()).host(CounterActor).config(config(secs(30))).build();
    assert_eq!(host.run_due_turns(10).await.unwrap(), 1);
    advance(secs(2)).await;
    // And a lost claim: a lease that runs out and is replaced.
    let first = one(server.lease_turns(&worker("w1"), &selection(), 10).await.unwrap());
    advance(secs(31)).await;
    let now = server.now().await.unwrap();

    let lease = one(server.lease_turns(&worker("w2"), &selection(), 10).await.unwrap());

    assert_eq!(lease.actor, address("c1"));
    assert!(lease.epoch > first.epoch);
    let m = &lease.message;
    assert_eq!((m.id.clone(), m.attempt, m.failures, m.lost_claims), (sent.id.clone(), 3, 1, 1));
    assert_eq!(m.last_error.as_deref(), Some("failed on purpose"));
    assert_eq!(
        (&m.correlation_id, &m.headers, &m.retry, &m.retention),
        (&sent.correlation_id, &sent.headers, &sent.retry, &sent.retention)
    );
    assert_eq!(&lease.message, &pending_head(&server, "c1").await, "the stored message as it is");
    let snapshot = server.get_actor(&address("c1")).await.unwrap().unwrap();
    assert_eq!((&lease.state, lease.version), (&snapshot.state, snapshot.version));
    assert_eq!(lease.state, json!({"total": 0}));
    assert_eq!((lease.claimed_at, lease.lease_until, lease.ttl), (now, plus(now, secs(30)), secs(30)));

    assert_eq!(server.state(), NodeState::New);
    assert!(server.hosted_types().is_empty());
    assert_eq!(server.running_turns(), 0, "a remote lease is not a local turn");
    assert!(server.nodes().await.unwrap().is_empty(), "no node row for the worker");
    assert_eq!(server.running_turn(&address("c1")).await.unwrap().expect("running").node, worker("w2"));
}

/// Scenario: Invalid requests.
#[tokio::test(start_paused = true)]
async fn invalid_requests_are_refused_before_any_store_access() {
    let store = store();
    let server = server(&store, secs(30));
    setup(&server, &["c1"]).await;
    let m = send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let too_many: BTreeMap<TypeName, String> =
        (0..=MAX_LEASE_TYPES).map(|i| (name(&format!("t{i}")), "h".to_owned())).collect();
    assert_eq!((MAX_LEASE_BATCH, MAX_LEASE_TYPES), (100, 100));
    let cases = [
        (worker("w"), BTreeMap::new(), 1),
        (worker("w"), too_many, 1),
        (worker("w"), selection(), 0),
        (worker("w"), selection(), MAX_LEASE_BATCH + 1),
        (worker(&"x".repeat(201)), selection(), 1),
        (worker("a\0b"), selection(), 1),
    ];
    for (owner, types, limit) in &cases {
        assert_invalid(server.lease_turns(owner, types, *limit).await);
    }
    let Some(MessageStatus::Pending(waiting)) = server.status(&m.id).await.unwrap() else { panic!("pending") };
    assert_eq!(waiting.attempt, 0, "nothing was claimed");

    store.close().await;
    for (owner, types, limit) in &cases {
        assert_invalid(server.lease_turns(owner, types, *limit).await);
    }
}

/// Scenario: Invalid requests, at the bounds.
#[tokio::test(start_paused = true)]
async fn an_owner_of_200_characters_and_a_limit_of_100_are_accepted() {
    let server = server(&store(), secs(30));
    setup(&server, &["c1"]).await;
    send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let owner = worker(&"x".repeat(200));
    let lease = one(server.lease_turns(&owner, &selection(), MAX_LEASE_BATCH).await.unwrap());
    assert_eq!(server.running_turn(&lease.actor).await.unwrap().unwrap().node, owner);
}

/// Scenario: Contract mismatch. Scenario: Contract replacement between requests.
#[tokio::test(start_paused = true)]
async fn a_type_that_is_absent_or_has_another_hash_refuses_the_request_and_claims_nothing() {
    let server = server(&store(), secs(30));
    setup(&server, &["c1"]).await;
    let m = send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let registered = contract_of::<CounterActor>().unwrap().hash().to_owned();

    let mut types = BTreeMap::from([(name("counter"), "sha256:old".to_owned()), (name("zeta"), "sha256:z".to_owned())]);
    types.insert(name("alpha"), "sha256:a".to_owned());
    assert_eq!(
        server.lease_turns(&worker("w"), &types, 10).await.unwrap(),
        LeaseTurnsResult::ContractMismatch { type_name: name("alpha"), registered: None, requested: "sha256:a".into() },
        "the first mismatch in name order"
    );
    types.remove(&name("alpha"));
    assert_eq!(
        server.lease_turns(&worker("w"), &types, 10).await.unwrap(),
        LeaseTurnsResult::ContractMismatch {
            type_name: name("counter"),
            registered: Some(registered),
            requested: "sha256:old".into()
        }
    );
    let Some(MessageStatus::Pending(waiting)) = server.status(&m.id).await.unwrap() else { panic!("pending") };
    assert_eq!(waiting.attempt, 0, "nothing was claimed");

    // An unused type replaced between two requests: the old hash no longer leases.
    let spare = |state: serde_json::Value| {
        Contract::new(name("spare"), state, BTreeMap::from([("Go".to_owned(), json!({"type": "object"}))]))
    };
    let (old, new) = (spare(json!({"type": "object"})), spare(json!({"type": "object", "required": ["n"]})));
    server.register_contract(&old).await.unwrap();
    let by_old = BTreeMap::from([(name("spare"), old.hash().to_owned())]);
    assert_eq!(server.lease_turns(&worker("w"), &by_old, 10).await.unwrap(), LeaseTurnsResult::Leased(Vec::new()));
    server.remove_type(&name("spare")).await.unwrap();
    server.register_contract(&new).await.unwrap();
    assert_eq!(
        server.lease_turns(&worker("w"), &by_old, 10).await.unwrap(),
        LeaseTurnsResult::ContractMismatch {
            type_name: name("spare"),
            registered: Some(new.hash().to_owned()),
            requested: old.hash().to_owned()
        }
    );
}

/// Scenario: Local and remote contenders.
#[tokio::test(start_paused = true)]
async fn local_nodes_and_external_callers_share_lease_exclusivity_and_delivery_order() {
    let store = store();
    let server = server(&store, secs(30));
    setup(&server, &["c1", "c2", "c3"]).await;
    let head = send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    send(&server, "c1", CounterMsg::Add { n: 2 }).await;
    send(&server, "c2", CounterMsg::Add { n: 3 }).await;
    send(&server, "c3", CounterMsg::Add { n: 4 }).await;
    server.pause_mailbox(&address("c3")).await.unwrap();
    let host = ActorSystem::builder(store.clone()).host(CounterActor).config(config(secs(30))).build();

    let leases = lease(&server, "w1").await;

    let actors: Vec<_> = leases.iter().map(|l| l.actor.clone()).collect();
    assert_eq!(actors, vec![address("c1"), address("c2")], "one head per actor, no paused actor");
    assert_eq!(leases[0].message.id, head.id);
    assert!(lease(&server, "w2").await.is_empty(), "another caller gets nothing");
    assert_eq!(host.run_due_turns(10).await.unwrap(), 0, "nor does a local node");

    server.release_turn(&leases[0].actor, leases[0].epoch).await.unwrap();
    assert_eq!(host.run_due_turns(10).await.unwrap(), 1, "the local node runs the released turn");
    assert_eq!(host.get_actor(&address("c1")).await.unwrap().unwrap().state, json!({"total": 1}));
    assert_eq!(host.run_due_turns(10).await.unwrap(), 1, "and the next one");
    let locally = host.run_due_turns(10).await.unwrap();
    assert_eq!(locally, 0, "c2 stays leased, c3 paused");
}

/// Scenario: Worker visibility.
#[tokio::test(start_paused = true)]
async fn a_worker_shows_as_the_owner_of_its_turns_without_a_node_row() {
    let store = store();
    let server = server(&store, secs(30));
    setup(&server, &["c1", "c2"]).await;
    send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let first = one(server.lease_turns(&worker("w1"), &selection(), 1).await.unwrap());

    let turns = server.list_running_turns(Some(&worker("w1")), "", None, 10).await.unwrap().items;
    assert_eq!(turns.iter().map(|t| &t.actor).collect::<Vec<_>>(), vec![&first.actor]);
    assert_eq!(turns[0].message_id, first.message.id);
    assert!(server.nodes().await.unwrap().is_empty(), "no node row is invented");

    // A worker may reuse the id of a node: their running turns then show together.
    let shared = worker("shared");
    let node = ActorSystem::builder(store.clone()).config(NodeConfig { node_id: shared.clone(), ..config(secs(30)) });
    let node = node.build();
    node.start().await.unwrap();
    send(&server, "c2", CounterMsg::Add { n: 2 }).await;
    let second = one(server.lease_turns(&shared, &selection(), 10).await.unwrap());
    let info = server.node(&shared).await.unwrap().expect("the node's row");
    assert_eq!(info.running_turns.iter().map(|t| &t.actor).collect::<Vec<_>>(), vec![&second.actor]);
    node.stop().await;
}

/// Scenario: A shorter renewal response is withheld. Scenario: Different servers grant different TTLs.
#[tokio::test(start_paused = true)]
async fn a_shorter_renewal_through_a_peer_keeps_the_lease_until_its_first_expiry() {
    let store = store();
    let (a, b, c) = (server(&store, secs(60)), server(&store, secs(5)), server(&store, secs(60)));
    setup(&a, &["c1"]).await;
    send(&a, "c1", CounterMsg::Add { n: 1 }).await;
    let t0 = a.now().await.unwrap();
    let held = one(a.lease_turns(&worker("w1"), &selection(), 10).await.unwrap());
    assert_eq!((held.lease_until, held.ttl), (plus(t0, secs(60)), secs(60)));

    advance(secs(10)).await;
    // B's answer may never reach the worker; the lease it leaves is the one the worker knows.
    assert_eq!(renewed(b.renew_turn(&held.actor, held.epoch).await.unwrap()), (plus(t0, secs(60)), secs(5), false));

    advance(secs(5)).await;
    assert!(lease(&c, "w2").await.is_empty(), "not claimable at t=15");
    advance(secs(44)).await;
    assert!(lease(&c, "w2").await.is_empty(), "nor at t=59");
    advance(secs(1)).await;
    let next = one(c.lease_turns(&worker("w2"), &selection(), 10).await.unwrap());
    assert_eq!((next.message.lost_claims, next.message.attempt), (1, 2), "claimable at t=60, the first claim lost");
}

/// Scenario: Different servers grant different TTLs. Scenario: Renewal through a peer.
#[tokio::test(start_paused = true)]
async fn a_longer_renewal_through_a_peer_extends_the_lease_and_returns_its_ttl() {
    let store = store();
    let (a, d) = (server(&store, secs(60)), server(&store, secs(80)));
    setup(&a, &["c1"]).await;
    send(&a, "c1", CounterMsg::Add { n: 1 }).await;
    let held = one(a.lease_turns(&worker("w1"), &selection(), 10).await.unwrap());
    advance(secs(10)).await;
    let now = d.now().await.unwrap();

    assert_eq!(renewed(d.renew_turn(&held.actor, held.epoch).await.unwrap()), (plus(now, secs(80)), secs(80), false));

    assert_eq!(a.running_turn(&held.actor).await.unwrap().unwrap().lease_until, plus(now, secs(80)));
    d.release_turn(&held.actor, held.epoch).await.unwrap();
    assert!(a.running_turn(&held.actor).await.unwrap().is_none(), "a peer releases it too");
}

/// Scenario: Idempotent release. Scenario: Stale or released epochs.
#[tokio::test(start_paused = true)]
async fn a_release_repeats_harmlessly_and_a_stale_epoch_touches_no_later_lease() {
    let server = server(&store(), secs(30));
    setup(&server, &["c1"]).await;
    let m = send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let first = one(server.lease_turns(&worker("w"), &selection(), 10).await.unwrap());

    server.release_turn(&first.actor, first.epoch).await.unwrap();
    server.release_turn(&first.actor, first.epoch).await.unwrap();
    let Some(MessageStatus::Pending(waiting)) = server.status(&m.id).await.unwrap() else { panic!("pending") };
    assert_eq!(waiting.attempt, 1, "released, its attempt counted");

    let second = one(server.lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    assert_eq!((second.message.attempt, second.message.lost_claims), (2, 0), "a release is no lost claim");
    assert!(second.epoch > first.epoch);
    assert_eq!(server.renew_turn(&first.actor, first.epoch).await.unwrap(), RenewTurnResult::LeaseLost);
    server.release_turn(&first.actor, first.epoch).await.unwrap();
    let running = server.running_turn(&second.actor).await.unwrap().expect("the later lease stays");
    assert_eq!(running.lease_until, second.lease_until);
    renewed(server.renew_turn(&second.actor, second.epoch).await.unwrap());
}

/// Scenario: Expired but not replaced.
#[tokio::test(start_paused = true)]
async fn an_expired_lease_nobody_replaced_still_renews_and_releases() {
    let server = server(&store(), secs(30));
    setup(&server, &["c1"]).await;
    let m = send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let held = one(server.lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    advance(secs(40)).await;
    assert!(matches!(server.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(_))), "expired");
    let now = server.now().await.unwrap();

    assert_eq!(
        renewed(server.renew_turn(&held.actor, held.epoch).await.unwrap()),
        (plus(now, secs(30)), secs(30), false)
    );
    assert!(matches!(server.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })));

    advance(secs(40)).await;
    server.release_turn(&held.actor, held.epoch).await.unwrap();
    let next = one(server.lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    assert_eq!(next.message.lost_claims, 0, "released, not lost");
}

/// Scenario: Stale or released epochs, after a delete and a create.
#[tokio::test(start_paused = true)]
async fn an_epoch_of_a_deleted_actor_cannot_touch_the_lease_of_its_next_life() {
    let server = server(&store(), secs(30));
    setup(&server, &["c1"]).await;
    send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let old = one(server.lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    server.delete_actor(&old.actor).await.unwrap();
    assert_eq!(server.renew_turn(&old.actor, old.epoch).await.unwrap(), RenewTurnResult::LeaseLost);

    server.create(&counter("c1"), &CounterState::default()).await.unwrap();
    send(&server, "c1", CounterMsg::Add { n: 2 }).await;
    let new = one(server.lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    assert!(new.epoch > old.epoch);
    assert_eq!(server.renew_turn(&old.actor, old.epoch).await.unwrap(), RenewTurnResult::LeaseLost);
    server.release_turn(&old.actor, old.epoch).await.unwrap();
    assert_eq!(server.running_turn(&new.actor).await.unwrap().expect("held").lease_until, new.lease_until);
}

/// Scenario: Operator cancellation.
#[tokio::test(start_paused = true)]
async fn a_renewal_reports_a_cancel_request_and_ends_nothing() {
    let server = server(&store(), secs(30));
    setup(&server, &["c1"]).await;
    let m = send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let held = one(server.lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    assert!(server.cancel_turn(&held.actor).await.unwrap());
    advance(secs(1)).await;

    let (until, _, cancel_requested) = renewed(server.renew_turn(&held.actor, held.epoch).await.unwrap());

    assert!(cancel_requested);
    assert!(until > held.lease_until, "the lease is extended all the same");
    assert!(matches!(server.status(&m.id).await.unwrap(), Some(MessageStatus::Running { .. })));
    server.release_turn(&held.actor, held.epoch).await.unwrap();
    assert!(
        matches!(server.status(&m.id).await.unwrap(), Some(MessageStatus::Pending(_))),
        "a release leaves it pending"
    );
}

/// Scenario: Stopped server. Scenario: Existing holders during stop.
#[tokio::test(start_paused = true)]
async fn a_stopped_system_leases_nothing_and_still_renews_and_releases() {
    let store = store();
    let peer = server(&store, secs(30));
    setup(&peer, &["c1"]).await;
    send(&peer, "c1", CounterMsg::Add { n: 1 }).await;
    let system = server(&store, secs(30));
    system.start().await.unwrap();
    let held = one(system.lease_turns(&worker("w"), &selection(), 10).await.unwrap());

    system.stop().await;

    assert_eq!(system.state(), NodeState::Stopped);
    assert_unavailable(system.lease_turns(&worker("w"), &selection(), 10).await);
    let running = peer.running_turn(&held.actor).await.unwrap().expect("stop leaves the lease alone");
    assert_eq!(running.node, worker("w"));
    renewed(system.renew_turn(&held.actor, held.epoch).await.unwrap());
    renewed(peer.renew_turn(&held.actor, held.epoch).await.unwrap());
    system.release_turn(&held.actor, held.epoch).await.unwrap();
    assert!(peer.running_turn(&held.actor).await.unwrap().is_none());

    let never_started = server(&store, secs(30));
    never_started.stop().await;
    assert_unavailable(never_started.lease_turns(&worker("w"), &selection(), 10).await);
}

/// Scenario: Invalid server configuration.
#[tokio::test(start_paused = true)]
async fn an_invalid_configuration_refuses_lease_and_renewal_and_still_releases() {
    let store = store();
    let peer = server(&store, secs(30));
    setup(&peer, &["c1"]).await;
    send(&peer, "c1", CounterMsg::Add { n: 1 }).await;
    let held = one(peer.lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    // It hosts an actor whose definition is never checked: only the configuration is.
    let broken = ActorSystem::builder(store.clone()).host(CounterActor).config(config(Duration::ZERO)).build();

    assert!(matches!(broken.lease_turns(&worker("w"), &selection(), 10).await, Err(OpError::Start(_))));
    assert!(matches!(broken.renew_turn(&held.actor, held.epoch).await, Err(OpError::Start(_))));
    broken.release_turn(&held.actor, held.epoch).await.unwrap();
    assert!(peer.running_turn(&held.actor).await.unwrap().is_none());
    assert_eq!(broken.state(), NodeState::New);
}

/// Scenario: Closed store.
#[tokio::test(start_paused = true)]
async fn a_closed_store_answers_unavailable() {
    let store = store();
    let server = server(&store, secs(30));
    setup(&server, &["c1"]).await;
    send(&server, "c1", CounterMsg::Add { n: 1 }).await;
    let held = one(server.lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    store.close().await;

    assert_unavailable(server.lease_turns(&worker("w"), &selection(), 10).await);
    assert_unavailable(server.renew_turn(&held.actor, held.epoch).await);
    assert_unavailable(server.release_turn(&held.actor, held.epoch).await);
}

/// Scenario: Deadline overflow.
#[tokio::test(start_paused = true)]
async fn a_ttl_beyond_the_clocks_is_invalid_without_a_claim() {
    let store = store();
    let server = server(&store, Duration::MAX);
    setup(&server, &["c1"]).await;
    let m = send(&server, "c1", CounterMsg::Add { n: 1 }).await;

    assert_invalid(server.lease_turns(&worker("w"), &selection(), 10).await);
    let Some(MessageStatus::Pending(waiting)) = server.status(&m.id).await.unwrap() else { panic!("pending") };
    assert_eq!(waiting.attempt, 0, "nothing was claimed");

    // The store clock cannot add it either: the store's own error.
    let held = one(self::server(&store, secs(30)).lease_turns(&worker("w"), &selection(), 10).await.unwrap());
    let renewal = server.renew_turn(&held.actor, held.epoch).await;
    assert!(matches!(renewal, Err(OpError::Store(StoreError::Invalid(_)))), "{renewal:?}");
}
