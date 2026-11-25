//! The remote turn routes (`POST /leases`, `/leases/renew`, `/leases/release`, `/leases/complete`; spec
//! `remote-turn-http`, ADR 0040), through router oneshot requests on virtual time: handler-free, unstarted, peer,
//! stopped and closed systems, hash mismatches, release idempotence, fencing, explicit core alternatives, the
//! route-specific parser (duplicate keys, empty, trailing and malformed bodies, size and depth limits), the read
//! router, and which observers hear of what.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use cicada::actor::contract_of;
use cicada::api::problem_types as pt;
use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
use cicada::prelude::*;
use cicada::runtime::NodeConfig;
use cicada::store::{MessageStatus, OutcomeStatus};
use cicada::{FailureCause, Store, TypeName};
use cicada_axum::{ApiConfig, actor_api, actor_api_read, actor_api_with};
use common::*;
use serde_json::{Value, json};

/// Counts the turns a server's observers hear of.
#[derive(Default, Clone)]
struct Heard {
    started: Arc<AtomicUsize>,
    ended: Arc<AtomicUsize>,
}

struct HeardTurn(Arc<AtomicUsize>);

impl TurnObserver for Heard {
    fn turn_started(&self, _turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        self.started.fetch_add(1, Ordering::SeqCst);
        Box::new(HeardTurn(self.ended.clone()))
    }
}

impl ObservedTurn for HeardTurn {
    fn turn_ended(self: Box<Self>, _end: &TurnEnd<'_>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn config(ttl: Duration) -> NodeConfig {
    NodeConfig { lease_ttl: ttl, ..NodeConfig::default() }
}

/// An unstarted system that hosts nothing, with the counter contract registered.
async fn server(store: &Store, config: NodeConfig, heard: &Heard) -> ActorSystem {
    let system = ActorSystem::builder(store.clone()).config(config).observe(heard.clone()).build();
    system.register_contract(&contract_of::<CounterActor>().unwrap()).await.unwrap();
    system
}

fn hash() -> String {
    contract_of::<CounterActor>().unwrap().hash().to_owned()
}

fn lease_body(limit: usize) -> String {
    json!({"owner": "worker-1", "types": {"counter": hash()}, "limit": limit}).to_string()
}

/// Creates `counter/<id>` and sends it `Add(n)`; the message id.
async fn pending_add(system: &ActorSystem, id: &str, n: i32) -> MessageId {
    system.create(&Counter { id: id.into() }, &CounterState::default()).await.unwrap();
    match system.send(&Counter { id: id.into() }, CounterMsg::Add { n }).await.unwrap() {
        cicada::SendResult::Accepted(m) => m.id,
        other => panic!("not accepted: {other:?}"),
    }
}

/// Leases through `app` and returns the one lease.
async fn lease_one(app: &Router) -> Value {
    let reply = post(app, "/leases", &lease_body(10)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let body = reply.json();
    assert_eq!(body["result"], "leased", "{body}");
    let leases = body["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1, "{body}");
    leases[0].clone()
}

fn identity(lease: &Value) -> String {
    json!({"actor": lease["actor"], "epoch": lease["epoch"]}).to_string()
}

fn complete_body(lease: &Value, report: Value) -> String {
    json!({"actor": lease["actor"], "epoch": lease["epoch"], "messageId": lease["message"]["id"], "report": report})
        .to_string()
}

fn returned(state: Value, effects: Value) -> Value {
    json!({"kind": "returned", "state": state, "effects": effects})
}

async fn status_of(system: &ActorSystem, id: &MessageId) -> MessageStatus {
    system.status(id).await.unwrap().expect("the message is known")
}

fn is_pending(status: &MessageStatus) -> bool {
    matches!(status, MessageStatus::Pending(_))
}

/// Scenarios: Lease request and runnable batch; A complete execution lease; Lifecycle and access (an unstarted
/// handler-free system leases).
#[tokio::test(start_paused = true)]
async fn an_unstarted_handler_free_server_leases_a_complete_turn() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let app = actor_api(system.clone());
    let empty = post(&app, "/leases", &lease_body(10)).await;
    assert_eq!(empty.json(), json!({"result": "leased", "leases": []}), "an empty batch, not a long poll");

    let id = pending_add(&system, "a b/c", 5).await;
    let lease = lease_one(&app).await;
    assert_eq!(lease["actor"], "counter/a%20b%2Fc", "the canonical address, not encoded again");
    assert!(lease["epoch"].is_i64());
    assert_eq!(lease["state"], json!({"total": 0}));
    assert_eq!(lease["version"], 0);
    assert_eq!(lease["ttl"], "PT30S");
    for field in ["claimedAt", "leaseUntil"] {
        assert!(lease[field].is_string(), "{field}");
    }
    let message = lease["message"].as_object().unwrap();
    let fields: Vec<&str> = message.keys().map(String::as_str).collect();
    assert_eq!(
        fields,
        [
            "id",
            "to",
            "from",
            "type",
            "payload",
            "deliverAt",
            "correlationId",
            "headers",
            "seq",
            "attempt",
            "failures",
            "lostClaims",
            "lastError",
            "retry",
            "causedBy",
            "retention",
            "notification"
        ],
        "every field, nullable ones as null"
    );
    assert_eq!(message["id"], id.as_str());
    assert_eq!(message["payload"], json!({"n": 5}));
    assert_eq!((message["attempt"].clone(), message["lostClaims"].clone()), (json!(1), json!(0)));
    assert!(matches!(status_of(&system, &id).await, MessageStatus::Running { .. }));
    assert_eq!(system.state(), cicada::runtime::NodeState::New, "leasing does not start the server");
    assert_eq!(heard.started.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_contract_mismatch_is_a_result_and_leases_nothing() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "x", 1).await;
    let app = actor_api(system.clone());
    let reply =
        post(&app, "/leases", &json!({"owner": "w", "types": {"counter": "sha256:old"}, "limit": 5}).to_string()).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json!({"result": "contractMismatch", "type": "counter", "registered": hash(), "requested": "sha256:old"})
    );
    let reply =
        post(&app, "/leases", &json!({"owner": "w", "types": {"ghost": "sha256:x"}, "limit": 5}).to_string()).await;
    assert_eq!(
        reply.json(),
        json!({"result": "contractMismatch", "type": "ghost", "registered": null, "requested": "sha256:x"})
    );
    assert!(is_pending(&status_of(&system, &id).await), "nothing was claimed");
}

/// Scenarios: Renew and release through a peer; Completed response; Lifecycle and access (a peer continues the held
/// turn after the first server stopped).
#[tokio::test(start_paused = true)]
async fn a_peer_renews_and_completes_a_lease_another_server_granted_after_it_stopped() {
    let store = memory_store();
    let heard = Heard::default();
    let first = server(&store, config(Duration::from_secs(30)), &heard).await;
    let peer_heard = Heard::default();
    let peer = server(&store, config(Duration::from_secs(10)), &peer_heard).await;
    let id = pending_add(&first, "c1", 3).await;
    first.create(&Counter { id: "target".into() }, &CounterState::default()).await.unwrap();
    let (a, b) = (actor_api(first.clone()), actor_api(peer.clone()));
    let lease = lease_one(&a).await;
    first.stop().await;
    assert_problem(&post(&a, "/leases", &lease_body(1)).await, 503, pt::STORE_UNAVAILABLE);

    let renewed = post(&b, "/leases/renew", &identity(&lease)).await;
    assert_eq!(renewed.status, StatusCode::OK, "{}", renewed.text);
    let renewed = renewed.json();
    assert_eq!((renewed["result"].clone(), renewed["ttl"].clone()), (json!("renewed"), json!("PT10S")));
    assert_eq!(renewed["cancelRequested"], false);
    assert_eq!(renewed["leaseUntil"], lease["leaseUntil"], "a shorter TTL never shortens the lease");
    assert!(peer.cancel_turn(&"counter/c1".parse().unwrap()).await.unwrap());
    assert_eq!(post(&b, "/leases/renew", &identity(&lease)).await.json()["cancelRequested"], true);

    let report = returned(
        json!({"kind": "set", "value": {"total": 3}}),
        json!({"sends": [{"id": "sent-1", "to": "counter/target", "type": "Add", "payload": {"n": 1},
            "headers": {"h": "v"}}]}),
    );
    let reply = post(&b, "/leases/complete", &complete_body(&lease, report.clone())).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let completed = reply.json();
    assert_eq!(completed["result"], "completed");
    assert_eq!(completed["outcome"], json!({"kind": "done"}));
    assert_eq!(completed["cause"], Value::Null);
    assert_eq!(completed["version"], 1);
    assert_eq!(completed["cancelled"], json!([]));
    let sent = &completed["sent"][0];
    assert_eq!((sent["id"].clone(), sent["from"].clone()), (json!("sent-1"), json!("counter/c1")));
    assert_eq!(sent["causedBy"], id.as_str(), "the server sets the sender and the cause");
    assert_eq!(sent["headers"], json!({"h": "v"}));
    assert_eq!(first.outcome(&id).await.unwrap().unwrap().status, OutcomeStatus::Done);

    // A repeat, and a stale epoch, are lease-lost results.
    let again = post(&b, "/leases/complete", &complete_body(&lease, report)).await;
    assert_eq!(again.json(), json!({"result": "leaseLost"}));
    assert_eq!(post(&b, "/leases/renew", &identity(&lease)).await.json(), json!({"result": "leaseLost"}));
    assert_eq!(heard.ended.load(Ordering::SeqCst) + peer_heard.ended.load(Ordering::SeqCst), 0, "no observer heard");
}

#[tokio::test(start_paused = true)]
async fn release_answers_204_also_when_repeated_or_stale_and_never_touches_a_later_lease() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "r", 1).await;
    let app = actor_api(system.clone());
    let first = lease_one(&app).await;
    for _ in 0..2 {
        let reply = post(&app, "/leases/release", &identity(&first)).await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        assert!(reply.text.is_empty());
    }
    assert!(is_pending(&status_of(&system, &id).await));
    let second = lease_one(&app).await;
    assert_ne!(second["epoch"], first["epoch"]);
    assert_eq!(second["message"]["attempt"], 2);
    assert_eq!(post(&app, "/leases/release", &identity(&first)).await.status, StatusCode::NO_CONTENT);
    assert!(matches!(status_of(&system, &id).await, MessageStatus::Running { .. }), "the stale release did nothing");
    // Fencing: the old epoch cannot complete the new lease.
    let stale = post(&app, "/leases/complete", &complete_body(&first, json!({"kind": "cancelled"}))).await;
    assert_eq!(stale.json(), json!({"result": "leaseLost"}));
    assert!(matches!(status_of(&system, &id).await, MessageStatus::Running { .. }));
}

/// Scenario: Rejected response and permanent transition.
#[tokio::test(start_paused = true)]
async fn a_rejected_commit_is_a_result_and_commits_nothing_until_the_failure_report() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "j", 1).await;
    let app = actor_api(system.clone());
    let lease = lease_one(&app).await;
    let report = returned(
        json!({"kind": "unchanged"}),
        json!({"sends": [{"id": "s1", "to": "counter/absent", "type": "Add", "payload": {"n": 1}}]}),
    );
    let reply = post(&app, "/leases/complete", &complete_body(&lease, report)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let rejected = reply.json();
    assert_eq!(rejected["result"], "rejected", "{rejected}");
    assert_eq!(rejected["failure"]["cause"], "commit");
    assert_eq!(rejected["failure"]["retryable"], false);
    assert!(matches!(status_of(&system, &id).await, MessageStatus::Running { .. }), "nothing applied, still held");
    let failure = json!({"kind": "failed", "cause": rejected["failure"]["cause"],
        "retryable": rejected["failure"]["retryable"], "reason": rejected["failure"]["reason"]});
    let reply = post(&app, "/leases/complete", &complete_body(&lease, failure)).await;
    let completed = reply.json();
    assert_eq!(completed["result"], "completed", "{completed}");
    assert_eq!(completed["cause"], "commit");
    assert_eq!(completed["outcome"]["kind"], "failed");
    assert_eq!(system.outcome(&id).await.unwrap().unwrap().status, OutcomeStatus::Failed);
}

/// Scenario: Body and value depth limits (a value within the parser budget reaches core validation).
#[tokio::test(start_paused = true)]
async fn a_state_too_deep_to_store_fails_the_turn_and_one_too_deep_to_parse_is_400() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "deep", 1).await;
    let app = actor_api(system.clone());
    let lease = lease_one(&app).await;
    let nested = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let body = |depth: usize| {
        format!(
            r#"{{"actor":{},"epoch":{},"messageId":{},"report":{{"kind":"returned","state":{{"kind":"set","value":{}}},"effects":{{}}}}}}"#,
            lease["actor"],
            lease["epoch"],
            lease["message"]["id"],
            nested(depth)
        )
    };
    assert_problem(&post(&app, "/leases/complete", &body(200)).await, 400, pt::BAD_REQUEST);
    assert!(matches!(status_of(&system, &id).await, MessageStatus::Running { .. }), "nothing ran");
    let reply = post(&app, "/leases/complete", &body(100)).await;
    let completed = reply.json();
    assert_eq!(completed["result"], "completed", "{completed}");
    assert_eq!(completed["cause"], "handler");
    assert!(completed["outcome"]["error"].as_str().unwrap().starts_with("State cannot be stored:"), "{completed}");
}

#[tokio::test(start_paused = true)]
async fn operation_errors_keep_the_problem_mapping() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    pending_add(&system, "e", 1).await;
    let app = actor_api(system.clone());
    let invalid = [
        json!({"owner": "w", "types": {"counter": hash()}, "limit": 0}),
        json!({"owner": "w".repeat(201), "types": {"counter": hash()}, "limit": 1}),
        json!({"owner": "w", "types": {}, "limit": 1}),
    ];
    for body in invalid {
        assert_problem(&post(&app, "/leases", &body.to_string()).await, 400, pt::BAD_REQUEST);
    }
    let lease = lease_one(&app).await;
    let refused = json!({"kind": "failed", "cause": "refused", "retryable": true, "reason": "r"});
    assert_problem(&post(&app, "/leases/complete", &complete_body(&lease, refused)).await, 400, pt::BAD_REQUEST);
    let limit = json!({"kind": "failed", "cause": "claim-limit", "retryable": false, "reason": "r"});
    assert_problem(&post(&app, "/leases/complete", &complete_body(&lease, limit)).await, 400, pt::BAD_REQUEST);

    let broken = ActorSystem::builder(store.clone()).config(config(Duration::ZERO)).build();
    let broken_app = actor_api(broken);
    assert_problem(&post(&broken_app, "/leases", &lease_body(1)).await, 500, pt::INTERNAL_ERROR);
    assert_problem(&post(&broken_app, "/leases/renew", &identity(&lease)).await, 500, pt::INTERNAL_ERROR);

    store.close().await;
    assert_problem(&post(&app, "/leases", &lease_body(1)).await, 503, pt::STORE_UNAVAILABLE);
    assert_problem(&post(&app, "/leases/renew", &identity(&lease)).await, 503, pt::STORE_UNAVAILABLE);
    assert_problem(&post(&app, "/leases/release", &identity(&lease)).await, 503, pt::STORE_UNAVAILABLE);
    let cancelled = complete_body(&lease, json!({"kind": "cancelled"}));
    assert_problem(&post(&app, "/leases/complete", &cancelled).await, 503, pt::STORE_UNAVAILABLE);
}

/// A `400` for a repeated key.
fn assert_duplicate(reply: &Reply) {
    assert_problem(reply, 400, pt::BAD_REQUEST);
    assert!(reply.json()["detail"].as_str().unwrap().contains("duplicate key"), "{}", reply.text);
}

/// Scenario: Duplicate keys.
#[tokio::test(start_paused = true)]
async fn a_key_repeated_at_any_depth_is_refused_before_any_operation() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "d", 1).await;
    let app = actor_api(system.clone());
    let h = hash();
    // A JSON escape of the first letter: `counter` is `counter`, `limit` is `limit`.
    let backslash = char::from(92u8);
    let (c, l) = (format!("{backslash}u0063"), format!("{backslash}u006c"));
    let leases = [
        format!(r#"{{"owner":"w","owner":"v","types":{{"counter":"{h}"}},"limit":1}}"#),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}","counter":"{h}"}},"limit":1}}"#),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}","{c}ounter":"{h}"}},"limit":1}}"#),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}"}},"limit":1,"{l}imit":1}}"#),
    ];
    for body in &leases {
        assert_duplicate(&post(&app, "/leases", body).await);
    }
    assert!(is_pending(&status_of(&system, &id).await), "nothing was leased");

    let lease = lease_one(&app).await;
    let (actor, epoch, message) = (&lease["actor"], &lease["epoch"], &lease["message"]["id"]);
    let complete =
        |report: &str| format!(r#"{{"actor":{actor},"epoch":{epoch},"messageId":{message},"report":{report}}}"#);
    let effects =
        |send: &str| format!(r#"{{"kind":"returned","state":{{"kind":"unchanged"}},"effects":{{"sends":[{send}]}}}}"#);
    let bodies = [
        complete(r#"{"kind":"cancelled","kind":"cancelled"}"#),
        complete(r#"{"kind":"returned","state":{"kind":"set","value":{"a":1,"a":2}},"effects":{}}"#),
        complete(&format!(
            r#"{{"kind":"returned","state":{{"kind":"set","value":{{"x":[{{"deep":{{"a":1,"{backslash}u0061":2}}}}]}}}},"effects":{{}}}}"#
        )),
        complete(r#"{"kind":"returned","state":{"kind":"unchanged"},"effects":{"sends":[],"sends":[]}}"#),
        complete(&effects(r#"{"id":"s","to":"counter/d","type":"Add","payload":{"n":1,"n":2}}"#)),
        complete(&effects(r#"{"id":"s","to":"counter/d","type":"Add","payload":{},"headers":{"h":"1","h":"2"}}"#)),
        complete(&effects(r#"{"id":"s","id":"t","to":"counter/d","type":"Add","payload":{}}"#)),
        format!(
            r#"{{"actor":{actor},"actor":{actor},"epoch":{epoch},"messageId":{message},"report":{{"kind":"cancelled"}}}}"#
        ),
    ];
    for body in &bodies {
        assert_duplicate(&post(&app, "/leases/complete", body).await);
    }
    let renew = format!(r#"{{"actor":{actor},"epoch":{epoch},"epoch":{epoch}}}"#);
    assert_duplicate(&post(&app, "/leases/renew", &renew).await);
    assert_duplicate(&post(&app, "/leases/release", &renew).await);
    assert!(matches!(status_of(&system, &id).await, MessageStatus::Running { .. }), "not completed nor released");
    let ok = post(&app, "/leases/complete", &complete(r#"{"kind":"cancelled"}"#)).await;
    assert_eq!(ok.json()["result"], "completed", "the lease was still held: {}", ok.text);
}

/// Scenario: Required fields and variants.
#[tokio::test(start_paused = true)]
async fn malformed_bodies_are_400_before_any_operation_and_old_routes_keep_their_parsing() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "m", 1).await;
    let app = actor_api(system.clone());
    let h = hash();
    let leases = [
        String::new(),
        "  \n".to_owned(),
        "[]".to_owned(),
        r#""text""#.to_owned(),
        "null".to_owned(),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}"}},"limit":1}} {{}}"#),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}"}},"limit":1}}x"#),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}"}}}}"#),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}"}},"limit":1,"wait":"PT1S"}}"#),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}"}},"limit":"1"}}"#),
        format!(r#"{{"owner":"w","types":{{"counter":"{h}"}},"limit":1.0}}"#),
        format!(r#"{{"owner":null,"types":{{"counter":"{h}"}},"limit":1}}"#),
        "{".to_owned(),
    ];
    for body in &leases {
        assert_problem(&post(&app, "/leases", body).await, 400, pt::BAD_REQUEST);
    }
    assert!(is_pending(&status_of(&system, &id).await), "nothing was leased");
    let lease = lease_one(&app).await;
    let renews = [
        json!({"actor": lease["actor"]}),
        json!({"actor": lease["actor"], "epoch": lease["epoch"], "owner": "w"}),
        json!({"actor": lease["actor"], "epoch": 1.5}),
        json!({"actor": "Not An Address", "epoch": lease["epoch"]}),
    ];
    for body in &renews {
        assert_problem(&post(&app, "/leases/renew", &body.to_string()).await, 400, pt::BAD_REQUEST);
        assert_problem(&post(&app, "/leases/release", &body.to_string()).await, 400, pt::BAD_REQUEST);
    }
    let reports = [
        json!({"kind": "done"}),
        json!({"kind": "returned", "state": {"kind": "unchanged"}}),
        json!({"kind": "returned", "state": {"kind": "set"}, "effects": {}}),
        json!({"kind": "failed", "cause": "handler", "retryable": false}),
        json!({"kind": "cancelled", "outcome": "done"}),
        returned(json!({"kind": "unchanged"}), json!({"sends": [{"to": "counter/m", "type": "Add", "payload": {}}]})),
        returned(
            json!({"kind": "unchanged"}),
            json!({"sends": [{"id": "s", "to": "counter/m", "type": "Add", "payload": {}, "from": "counter/x"}]}),
        ),
        returned(
            json!({"kind": "unchanged"}),
            json!({"sends": [{"id": "s", "to": "counter/m", "type": "Add", "payload": {},
                "deliverAt": "2026-01-01T00:00:00Z", "delay": "PT1S"}]}),
        ),
        returned(json!({"kind": "unchanged"}), json!({"controls": [{"effect": "freeze", "actor": "counter/m"}]})),
        // `existing` on a type, or without `set` (ADR 0063)
        returned(
            json!({"kind": "unchanged"}),
            json!({"watches": [{"target": "counter", "set": true, "existing": true}]}),
        ),
        returned(
            json!({"kind": "unchanged"}),
            json!({"watches": [{"target": "counter/m", "set": false, "existing": true}]}),
        ),
    ];
    for report in reports {
        let reply = post(&app, "/leases/complete", &complete_body(&lease, report.clone())).await;
        assert_problem(&reply, 400, pt::BAD_REQUEST);
    }
    assert!(matches!(status_of(&system, &id).await, MessageStatus::Running { .. }), "nothing was completed");
    // An old route still takes a repeated key, its last value winning.
    let reply = post(&app, "/actors/counter/m/messages", r#"{"type":"Add","payload":{"n":1,"n":2}}"#).await;
    assert_eq!(reply.status, StatusCode::ACCEPTED, "{}", reply.text);
}

/// Scenario: Required fields and variants (a record given as a positional array is no record).
#[tokio::test(start_paused = true)]
async fn a_record_given_as_a_positional_array_is_400_before_any_operation() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "p", 1).await;
    system.create(&Counter { id: "victim".into() }, &CounterState::default()).await.unwrap();
    let app = actor_api(system.clone());
    let lease = lease_one(&app).await;
    let returned_with = |effects: Value| returned(json!({"kind": "unchanged"}), effects);
    let reports = [
        returned_with(json!([])),
        returned_with(json!({"controls": [["delete", "counter/victim"]]})),
        returned_with(json!({"creates": [["counter/new", {}, false]]})),
        returned_with(json!({"watches": [["*", true]]})),
        returned_with(json!({"sends": [["s", "counter/victim", "Add", {"n": 1}]]})),
        returned_with(json!({"sends": [{"id": "s", "to": "counter/victim", "type": "Add", "payload": {"n": 1},
            "retry": [2, {"kind": "fixed", "delay": "PT1S"}]}]})),
        returned(json!(["set", {"total": 9}]), json!({})),
        json!(["cancelled"]),
    ];
    for report in reports {
        let reply = post(&app, "/leases/complete", &complete_body(&lease, report.clone())).await;
        assert_problem(&reply, 400, pt::BAD_REQUEST);
    }
    assert!(matches!(status_of(&system, &id).await, MessageStatus::Running { .. }), "nothing was completed");
    assert!(system.get_actor(&"counter/victim".parse().unwrap()).await.unwrap().is_some(), "not deleted");
    assert!(system.get_actor(&"counter/new".parse().unwrap()).await.unwrap().is_none());
    for (path, body) in [("/leases/renew", json!([lease["actor"], lease["epoch"]])), ("/leases", json!(["w", {}, 1]))] {
        assert_problem(&post(&app, path, &body.to_string()).await, 400, pt::BAD_REQUEST);
    }
    let ok = post(&app, "/leases/complete", &complete_body(&lease, json!({"kind": "cancelled"}))).await;
    assert_eq!(ok.json()["result"], "completed", "the lease was still held: {}", ok.text);
}

/// Scenario: Body and value depth limits (size).
#[tokio::test(start_paused = true)]
async fn a_body_beyond_the_limit_is_413_with_or_without_a_length() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "big", 1).await;
    let app = actor_api_with(system.clone(), ApiConfig { max_request_body_bytes: 512 });
    let big = format!(r#"{{"owner":"{}","types":{{"counter":"{}"}},"limit":1}}"#, "w".repeat(600), hash());
    assert_problem(&post(&app, "/leases", &big).await, 413, pt::REQUEST_TOO_LARGE);
    let chunks = futures::stream::iter(
        big.into_bytes().chunks(64).map(|c| Ok::<_, std::io::Error>(c.to_vec())).collect::<Vec<_>>(),
    );
    let streamed = Request::builder()
        .method(Method::POST)
        .uri("/leases")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(chunks))
        .unwrap();
    assert!(streamed.headers().get(header::CONTENT_LENGTH).is_none(), "no length to refuse it by");
    assert_problem(&request(&app, streamed).await, 413, pt::REQUEST_TOO_LARGE);
    assert!(is_pending(&status_of(&system, &id).await), "nothing was leased");
}

/// Scenario: Lifecycle and access (the read router).
#[tokio::test(start_paused = true)]
async fn the_read_router_runs_no_lease_operation() {
    let store = memory_store();
    let heard = Heard::default();
    let system = server(&store, config(Duration::from_secs(30)), &heard).await;
    let id = pending_add(&system, "ro", 1).await;
    let read = actor_api_read(system.clone());
    let bodies = [
        ("/leases", lease_body(1)),
        ("/leases/renew", json!({"actor": "counter/ro", "epoch": 1}).to_string()),
        ("/leases/release", json!({"actor": "counter/ro", "epoch": 1}).to_string()),
        (
            "/leases/complete",
            json!({"actor": "counter/ro", "epoch": 1, "messageId": id.as_str(), "report": {"kind": "cancelled"}})
                .to_string(),
        ),
    ];
    for (path, body) in &bodies {
        assert_eq!(post(&read, path, body).await.status, StatusCode::NOT_FOUND, "{path}");
    }
    assert!(is_pending(&status_of(&system, &id).await));
    let document = get_json(&read, "/openapi.json").await;
    assert!(document["paths"].as_object().unwrap().keys().all(|p| !p.starts_with("/leases")));
}

/// Scenario: Lease request and runnable batch (claim-limit observers); Completion adds no server observer.
#[tokio::test(start_paused = true)]
async fn claim_limit_failures_are_observed_and_completions_are_not() {
    let store = memory_store();
    let heard = Heard::default();
    let limited = NodeConfig { max_lost_claims: Some(1), ..config(Duration::from_secs(30)) };
    let system = server(&store, limited, &heard).await;
    let lost = pending_add(&system, "lost", 1).await;
    let types = BTreeSet::from([TypeName::parse("counter").unwrap()]);
    cicada_test::claim_and_abandon(&store, &types, Duration::from_secs(1), 1).await.unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    let app = actor_api(system.clone());
    let reply = post(&app, "/leases", &lease_body(10)).await;
    assert_eq!(reply.json(), json!({"result": "leased", "leases": []}), "settled by the core, not handed out");
    let outcome = system.outcome(&lost).await.unwrap().unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert_eq!((heard.started.load(Ordering::SeqCst), heard.ended.load(Ordering::SeqCst)), (1, 1));

    pending_add(&system, "fresh", 1).await;
    let lease = lease_one(&app).await;
    let reply =
        post(&app, "/leases/complete", &complete_body(&lease, returned(json!({"kind": "stop"}), json!({})))).await;
    assert_eq!(reply.json()["result"], "completed");
    let failed = json!({"kind": "failed", "cause": "handler", "retryable": false, "reason": "r"});
    pending_add(&system, "fresh2", 1).await;
    let lease = lease_one(&app).await;
    let reply = post(&app, "/leases/complete", &complete_body(&lease, failed)).await;
    assert_eq!(reply.json()["cause"], json!(FailureCause::Handler.name()));
    assert_eq!((heard.started.load(Ordering::SeqCst), heard.ended.load(Ordering::SeqCst)), (1, 1), "HTTP adds none");
}
