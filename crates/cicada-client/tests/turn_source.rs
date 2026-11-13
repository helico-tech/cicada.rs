//! `ActorClient` as a `TurnSource` (spec `remote-turn-http`, ADR 0040): the classification of every status class by
//! the actual HTTP status, strict execution responses with a tolerant envelope, one request per call whatever
//! `send_attempts` says, the turn timeout through the body and a shorter timeout of the HTTP client, registration
//! conflicts and type listings. A scripted server answers over a real socket; timeouts are asserted on a paused clock
//! once the server holds the request.

mod common;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use cicada::actor::contract_of;
use cicada::address::TypeName;
use cicada::contract::Contract;
use cicada::runtime::{
    CompleteTurnResult, LeaseTurnsResult, RegisterContractResult, RenewTurnResult, SourceError, TurnFailure,
    TurnReport, TurnSource,
};
use cicada::store::{MessageId, NodeId, TypeDef};
use cicada::{ActorAddress, FailureCause};
use cicada_axum::actor_api;
use cicada_client::ActorClient;
use common::*;
use serde_json::{Value, json};
use tokio::sync::Notify;

/// How the scripted server answers one request.
#[derive(Clone)]
enum Script {
    /// This status and body (a problem when it starts with `{"type"`).
    Reply(u16, String),
    /// Holds the request without answering.
    Hang,
    /// Sends `200` and part of a body, then nothing more.
    PartialBody,
}

#[derive(Default)]
struct Scripted {
    script: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<(String, String)>>,
    received: Notify,
}

impl Scripted {
    fn then(&self, script: Script) {
        self.script.lock().unwrap().push_back(script);
    }

    fn requests(&self) -> Vec<(String, String)> {
        self.requests.lock().unwrap().clone()
    }
}

async fn answer(State(scripted): State<Arc<Scripted>>, req: Request) -> Response {
    let path = format!("{} {}", req.method(), req.uri().path());
    let body = axum::body::to_bytes(req.into_body(), usize::MAX).await.unwrap();
    scripted.requests.lock().unwrap().push((path, String::from_utf8(body.to_vec()).unwrap()));
    scripted.received.notify_one();
    let next = scripted.script.lock().unwrap().pop_front().unwrap_or(Script::Reply(500, String::new()));
    match next {
        Script::Reply(status, body) => {
            let content_type =
                if body.starts_with(r#"{"type""#) { cicada_axum::PROBLEM_JSON } else { "application/json" };
            (StatusCode::from_u16(status).unwrap(), [(header::CONTENT_TYPE, content_type)], body).into_response()
        }
        Script::Hang => std::future::pending().await,
        Script::PartialBody => {
            let first = futures::stream::once(async { Ok::<_, std::io::Error>(b"{\"result\":".to_vec()) });
            let body = Body::from_stream(futures::StreamExt::chain(first, futures::stream::pending()));
            (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], body).into_response()
        }
    }
}

/// A scripted server and a client of it that would retry a send five times.
async fn scripted() -> (Arc<Scripted>, ActorClient) {
    scripted_with(reqwest::Client::new()).await
}

async fn scripted_with(http: reqwest::Client) -> (Arc<Scripted>, ActorClient) {
    let state = Arc::new(Scripted::default());
    let app = Router::new().fallback(answer).with_state(state.clone());
    let base = serve(app).await;
    (state, ActorClient::with_client(base, http).send_attempts(5).retry_delay(Duration::from_millis(1)))
}

fn problem(_status: u16, problem_type: &str, claimed_status: u16) -> String {
    json!({"type": problem_type, "title": "T", "status": claimed_status, "detail": "d", "instance": "/x"}).to_string()
}

fn actor() -> ActorAddress {
    ActorAddress::parse("account/a%20b%2Fc").unwrap()
}

fn owner() -> NodeId {
    NodeId::new("w1").unwrap()
}

fn types() -> BTreeMap<TypeName, String> {
    BTreeMap::from([(TypeName::parse("account").unwrap(), "sha256:h".to_owned())])
}

fn kind(e: &SourceError) -> &'static str {
    match e {
        SourceError::Invalid(_) => "invalid",
        SourceError::Unavailable(_) => "unavailable",
        SourceError::Fatal(_) => "fatal",
    }
}

fn lease_json() -> Value {
    json!({"actor": "account/a%20b%2Fc", "epoch": i64::MIN, "state": {}, "version": i64::MAX,
        "claimedAt": "2026-01-01T00:00:00Z", "leaseUntil": "2026-01-01T00:00:30Z", "ttl": "PT0.000000002S",
        "message": {"id": "m1", "to": "account/a%20b%2Fc", "from": null, "type": "Deposit", "payload": {"amount": 1},
            "deliverAt": "2026-01-01T00:00:00Z", "correlationId": null, "headers": {}, "seq": i64::MAX, "attempt": 1,
            "failures": 0, "lostClaims": 0, "lastError": null, "retry": null, "causedBy": null, "retention": null,
            "notification": false}})
}

/// Scenario: Status classification (every class, by the actual status, with one request whatever send_attempts says).
#[tokio::test]
async fn every_status_class_maps_by_the_actual_http_status_after_one_request() {
    let (server, client) = scripted().await;
    let cases = [
        (400, "invalid"),
        (413, "invalid"),
        (422, "invalid"),
        (408, "unavailable"),
        (429, "unavailable"),
        (502, "unavailable"),
        (503, "unavailable"),
        (504, "unavailable"),
        (301, "fatal"),
        (307, "fatal"),
        (401, "fatal"),
        (403, "fatal"),
        (404, "fatal"),
        (405, "fatal"),
        (409, "fatal"),
        (500, "fatal"),
        (201, "fatal"),
    ];
    let mut calls = 0;
    for (status, expected) in cases {
        // The problem claims another status: only the HTTP status counts.
        let claimed = if status == 503 { 400 } else { 503 };
        let body = problem(status, "urn:cicada:problem:not-found", claimed);
        for op in 0..4 {
            server.then(Script::Reply(status, body.clone()));
            let error = match op {
                0 => client.lease_turns(&owner(), &types(), 1).await.map(|_| ()),
                1 => client.renew_turn(&actor(), 1).await.map(|_| ()),
                2 => client.release_turn(&actor(), 1).await,
                _ => client
                    .complete_turn(&actor(), 1, &MessageId::new("m").unwrap(), TurnReport::Cancelled)
                    .await
                    .map(|_| ()),
            }
            .expect_err("not a success");
            calls += 1;
            assert_eq!(kind(&error), expected, "HTTP {status}, operation {op}: {error}");
            assert_eq!(server.requests().len(), calls, "one request per call");
        }
    }
    // A release succeeds with 204 only, and the others with 200 only.
    server.then(Script::Reply(200, String::new()));
    assert_eq!(kind(&client.release_turn(&actor(), 1).await.unwrap_err()), "fatal");
    server.then(Script::Reply(204, String::new()));
    client.release_turn(&actor(), 1).await.unwrap();
    server.then(Script::Reply(204, String::new()));
    assert_eq!(kind(&client.renew_turn(&actor(), 1).await.unwrap_err()), "fatal");
}

/// Scenarios: Strict execution response and tolerant envelope; Exact numbers and addresses.
#[tokio::test]
async fn results_decode_strictly_with_extra_envelope_fields_ignored() {
    let (server, client) = scripted().await;
    let mut leased = json!({"result": "leased", "leases": [lease_json()], "extra": 1});
    leased["leases"][0]["extra"] = json!(true);
    server.then(Script::Reply(200, leased.to_string()));
    let LeaseTurnsResult::Leased(leases) = client.lease_turns(&owner(), &types(), 3).await.unwrap() else { panic!() };
    assert_eq!((leases[0].epoch, leases[0].version, leases[0].message.seq), (i64::MIN, i64::MAX, i64::MAX));
    assert_eq!(leases[0].ttl, Duration::from_nanos(2));
    assert_eq!(leases[0].actor, actor());
    let (path, body) = server.requests().pop().unwrap();
    assert_eq!(path, "POST /leases");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"owner": "w1", "types": {"account": "sha256:h"}, "limit": 3})
    );

    let mismatch = json!({"result": "contractMismatch", "type": "account", "registered": null, "requested": "x"});
    server.then(Script::Reply(200, mismatch.to_string()));
    assert!(matches!(
        client.lease_turns(&owner(), &types(), 1).await.unwrap(),
        LeaseTurnsResult::ContractMismatch { registered: None, .. }
    ));
    server.then(Script::Reply(200, json!({"result": "leaseLost"}).to_string()));
    assert_eq!(client.renew_turn(&actor(), i64::MAX).await.unwrap(), RenewTurnResult::LeaseLost);
    let (_, body) = server.requests().pop().unwrap();
    assert_eq!(
        body, r#"{"actor":"account/a%20b%2Fc","epoch":9223372036854775807}"#,
        "the address is not encoded again"
    );
    let rejected = json!({"result": "rejected", "failure": {"cause": "commit", "retryable": false, "reason": "r"}});
    server.then(Script::Reply(200, rejected.to_string()));
    let report = TurnReport::Failed(TurnFailure { cause: FailureCause::Handler, retryable: true, reason: "x".into() });
    let result = client.complete_turn(&actor(), 7, &MessageId::new("m").unwrap(), report).await.unwrap();
    assert_eq!(
        result,
        CompleteTurnResult::Rejected(TurnFailure { cause: FailureCause::Commit, retryable: false, reason: "r".into() })
    );
    let (path, body) = server.requests().pop().unwrap();
    assert_eq!(path, "POST /leases/complete");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"actor": "account/a%20b%2Fc", "epoch": 7, "messageId": "m",
            "report": {"kind": "failed", "cause": "handler", "retryable": true, "reason": "x"}})
    );
}

/// Scenario: Strict execution response and tolerant envelope (what is Fatal).
#[tokio::test]
async fn a_malformed_or_unknown_success_is_fatal_never_a_default() {
    let (server, client) = scripted().await;
    let with_message = |field: &str, value: Option<Value>| {
        let mut lease = lease_json();
        match value {
            Some(v) => lease["message"][field] = v,
            None => {
                lease["message"].as_object_mut().unwrap().remove(field);
            }
        }
        json!({"result": "leased", "leases": [lease]}).to_string()
    };
    let bodies = [
        "not json".to_owned(),
        String::new(),
        json!({"result": "leased"}).to_string(),
        json!({"result": "queued", "leases": []}).to_string(),
        with_message("retry", None),
        with_message("lostClaims", None),
        with_message("retry", Some(json!({"maxAttempts": 2, "backoff": {"kind": "fibonacci"}}))),
        with_message("retention", Some(json!({"done": {"kind": "forever"}}))),
        with_message("seq", Some(json!(1.0))),
    ];
    for body in bodies {
        server.then(Script::Reply(200, body.clone()));
        let error = client.lease_turns(&owner(), &types(), 1).await.unwrap_err();
        assert_eq!(kind(&error), "fatal", "{body}: {error}");
    }
    let completed = |outcome: Value, cause: Value| {
        json!({"result": "completed", "outcome": outcome, "cause": cause, "version": 1, "sent": [], "cancelled": []})
            .to_string()
    };
    for body in [
        completed(json!({"kind": "skipped"}), Value::Null),
        completed(json!({"kind": "failed", "error": "e"}), json!("gremlins")),
        completed(json!({"kind": "failed", "error": "e"}), Value::Null),
        completed(
            json!({"kind": "rescheduled", "error": "e", "delay": "PT1S", "deliverAt": "2026-01-01T00:00:00Z"}),
            Value::Null,
        ),
        json!({"result": "completed", "outcome": {"kind": "done"}, "version": 1, "sent": [], "cancelled": []})
            .to_string(),
    ] {
        server.then(Script::Reply(200, body.clone()));
        let error = client.complete_turn(&actor(), 1, &MessageId::new("m").unwrap(), TurnReport::Cancelled).await;
        assert_eq!(kind(&error.unwrap_err()), "fatal", "{body}");
    }
}

#[tokio::test]
async fn a_url_that_cannot_be_built_is_invalid_and_an_unreachable_server_unavailable() {
    let client = ActorClient::new("not a url");
    assert_eq!(kind(&client.renew_turn(&actor(), 1).await.unwrap_err()), "invalid");
    assert_eq!(kind(&client.list_types().await.unwrap_err()), "invalid");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let client = ActorClient::new(base).send_attempts(5);
    assert_eq!(kind(&client.lease_turns(&owner(), &types(), 1).await.unwrap_err()), "unavailable");
}

#[test]
#[should_panic(expected = "turn_timeout must be positive")]
fn a_zero_turn_timeout_is_refused() {
    let _ = ActorClient::new("http://localhost").turn_timeout(Duration::ZERO);
}

/// Scenario: Timeout and request count (a request held by the server, and a body that never ends).
#[tokio::test]
async fn the_turn_timeout_bounds_the_request_and_its_body_with_one_request() {
    for script in [Script::Hang, Script::PartialBody] {
        let (server, client) = scripted().await;
        let client = client.turn_timeout(Duration::from_secs(10));
        server.then(script);
        let call = tokio::spawn(async move { client.renew_turn(&actor(), 1).await });
        server.received.notified().await;
        tokio::time::pause();
        let paused = tokio::time::Instant::now();
        let error = call.await.unwrap().unwrap_err();
        let waited = paused.elapsed();
        tokio::time::resume();
        assert_eq!(kind(&error), "unavailable", "{error}");
        // The rest of the 10 s after the server got the request, up to the timer's millisecond rounding.
        assert!(waited > Duration::from_secs(9) && waited < Duration::from_millis(10_005), "{waited:?}");
        assert_eq!(server.requests().len(), 1, "no second request");
    }
}

/// Scenario: Supplied HTTP client policy (a shorter timeout of the HTTP client applies).
#[tokio::test]
async fn a_shorter_timeout_of_the_http_client_still_applies() {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(1)).build().unwrap();
    let (server, client) = scripted_with(http).await;
    server.then(Script::Hang);
    let call = tokio::spawn(async move { client.lease_turns(&owner(), &types(), 1).await });
    server.received.notified().await;
    tokio::time::pause();
    let paused = tokio::time::Instant::now();
    let error = call.await.unwrap().unwrap_err();
    let waited = paused.elapsed();
    tokio::time::resume();
    assert_eq!(kind(&error), "unavailable", "{error}");
    assert!(waited < Duration::from_millis(1_005), "the client's own 1 s, not the 10 s turn timeout: {waited:?}");
    assert_eq!(server.requests().len(), 1);
}

/// Scenario: Registration and type lookup (a conflict whose registration vanished; recognised alternatives only).
#[tokio::test]
async fn a_conflict_reads_the_registration_once_and_a_vanished_one_is_unavailable() {
    let (server, client) = scripted().await;
    let contract = contract_of::<AccountActor>().unwrap();
    server.then(Script::Reply(409, problem(409, cicada::api::problem_types::CONTRACT_CONFLICT, 409)));
    server.then(Script::Reply(404, problem(404, cicada::api::problem_types::NO_SUCH_TYPE, 404)));
    let error = client.register_contract(&contract).await.unwrap_err();
    assert_eq!(kind(&error), "unavailable", "{error}");
    let paths: Vec<String> = server.requests().into_iter().map(|(p, _)| p).collect();
    assert_eq!(paths, ["PUT /types/account", "GET /types/account"], "one GET, no second PUT");

    server.then(Script::Reply(422, problem(422, cicada::api::problem_types::INVALID_CONTRACT, 422)));
    assert!(
        matches!(client.register_contract(&contract).await.unwrap(), RegisterContractResult::InvalidContract(d) if d == "d")
    );
    // A 409 or 422 of another kind is no alternative of the core.
    server.then(Script::Reply(409, problem(409, cicada::api::problem_types::TYPE_IN_USE, 409)));
    assert_eq!(kind(&client.register_contract(&contract).await.unwrap_err()), "fatal");
    server.then(Script::Reply(422, problem(422, cicada::api::problem_types::INVALID_STATE, 422)));
    assert_eq!(kind(&client.register_contract(&contract).await.unwrap_err()), "invalid");
    server.then(Script::Reply(503, problem(503, cicada::api::problem_types::STORE_UNAVAILABLE, 503)));
    assert_eq!(kind(&client.register_contract(&contract).await.unwrap_err()), "unavailable");
    assert_eq!(server.requests().len(), 6, "never retried");
}

/// Scenario: Registration and type lookup (against a real server: a stale cached hash, served definitions).
#[tokio::test]
async fn registration_recomputes_the_requested_hash_and_listing_keeps_served_definitions() {
    let system = node_with(|b| b).await;
    let client = ActorClient::new(serve(actor_api(system.clone())).await);
    let contract = contract_of::<AccountActor>().unwrap();
    let RegisterContractResult::Registered(stored) = client.register_contract(&contract).await.unwrap() else {
        panic!("registered")
    };
    assert_eq!(stored.definition.contract_hash, contract.hash());
    assert_eq!(stored.contract.hash(), contract.hash());

    // A contract read from a registry entry keeps that entry's hash, whatever its schemas say.
    let other = Contract::new(
        contract.type_name().clone(),
        json!({"type": "object", "description": "changed"}),
        BTreeMap::new(),
    );
    let stale = Contract::of(&TypeDef { contract_hash: contract.hash().to_owned(), ..other.to_type_def() }).unwrap();
    assert_eq!(stale.hash(), contract.hash());
    let RegisterContractResult::Conflict { existing, requested_hash } = client.register_contract(&stale).await.unwrap()
    else {
        panic!("a conflict")
    };
    assert_eq!(requested_hash, other.hash(), "computed from the schemas, not the cached hash");
    assert_ne!(requested_hash, contract.hash());
    assert_eq!(existing.definition.contract_hash, contract.hash());

    let described = Contract::new(
        TypeName::parse("described").unwrap(),
        json!({"type": "object", "description": "The state."}),
        BTreeMap::from([("Go".to_owned(), json!({"type": "object", "description": "Go."}))]),
    );
    client.register_contract(&described).await.unwrap();
    let listed = client.list_types().await.unwrap();
    let served = listed.iter().find(|d| d.name.as_str() == "described").unwrap();
    assert_eq!(served.contract_hash, described.hash());
    assert_eq!(served.contract, described.to_json(), "descriptions kept");
    let from_system: Vec<TypeDef> = system.types().await.unwrap().into_iter().map(|t| t.definition).collect();
    assert_eq!(listed, from_system);
}

/// The management API keeps its own behaviour: a send is still retried on 503.
#[tokio::test]
async fn management_sends_keep_their_retries() {
    let (server, client) = scripted().await;
    for _ in 0..2 {
        server.then(Script::Reply(503, problem(503, cicada::api::problem_types::STORE_UNAVAILABLE, 503)));
    }
    server.then(Script::Reply(202, json!({"id": "m", "deliverAt": "2026-01-01T00:00:00Z"}).to_string()));
    client.send_message(&actor(), "Deposit", json!({"amount": 1}), Default::default()).await.unwrap();
    assert_eq!(server.requests().len(), 3);
}
