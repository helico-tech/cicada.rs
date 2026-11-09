//! The client against a real `actor_api()` server on a local port, in real time.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use cicada::ActorAddress;
use cicada::api::{MessageState, problem_types as pt};
use cicada::prelude::*;
use cicada::store::Delivery;
use cicada::time::parse_instant;
use cicada_axum::actor_api;
use cicada_client::{ActorClient, CancelOutcome, ClientError};
use common::*;
use serde_json::{Value, json};

fn acme() -> Account {
    account("acme", 7)
}

fn assert_problem<T: std::fmt::Debug>(result: Result<T, ClientError>, status: u16, problem_type: &str) {
    match result {
        Err(ClientError::Api(p)) => {
            assert_eq!(p.status, status, "{p:?}");
            assert_eq!(p.problem_type, problem_type, "{p:?}");
        }
        other => panic!("expected a {status} {problem_type} problem, got {other:?}"),
    }
}

fn hour() -> SendOptions {
    SendOptions::new().delay(Duration::from_secs(3600))
}

async fn balance(actors: &ActorClient, r: &Account) -> Option<i64> {
    actors.state::<_, AccountState>(r).await.unwrap().map(|s| s.balance)
}

#[tokio::test]
async fn typed_create_send_state_and_status() {
    let (actors, _, _) = client_test(|r| r).await;
    assert!(actors.create(&acme(), &owned_by("Ada")).await.unwrap().created);
    let again = actors.create(&acme(), &owned_by("Bob")).await.unwrap();
    assert!(!again.created);
    assert_eq!(again.uri, acme().address().unwrap());

    let receipt = actors.send(&acme(), AccountMsg::Deposit { amount: 5 }).await.unwrap();
    assert!(!receipt.duplicate);
    assert!(receipt.deliver_at.is_some());
    eventually(async || actors.status(&receipt.id).await.unwrap().map(|s| s.status) == Some(MessageState::Done)).await;
    assert_eq!(actors.state(&acme()).await.unwrap(), Some(AccountState { balance: 5, owner: "Ada".into() }));

    let view = actors.get(&acme()).await.unwrap().unwrap();
    assert_eq!(view.template, "actor://account/{tenant}/{id}");
    assert_eq!(view.params.iter().collect::<Vec<_>>(), vec![("tenant", "acme"), ("id", "7")]);
    assert_eq!(view.version, 1);

    assert!(actors.get(&account("acme", 8)).await.unwrap().is_none());
    assert!(actors.state::<_, AccountState>(&account("acme", 8)).await.unwrap().is_none());
}

#[tokio::test]
async fn chain_of_a_message_and_the_cause_in_its_envelope() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    actors.create(&account("acme", 8), &AccountState::default()).await.unwrap();
    let receipt = actors.send(&acme(), AccountMsg::Transfer { to: 8, amount: 3 }).await.unwrap();
    eventually(async || balance(&actors, &account("acme", 8)).await == Some(3)).await;

    let chain = actors.chain(&receipt.id).await.unwrap().unwrap();
    assert_eq!(chain.focus, receipt.id);
    assert!(!chain.truncated);
    assert_eq!(chain.nodes.iter().map(|n| n.message_type.as_str()).collect::<Vec<_>>(), ["Transfer", "Deposit"]);
    let deposit = chain.nodes.last().unwrap();
    assert_eq!(deposit.caused_by.as_ref(), Some(&receipt.id));
    assert_eq!(deposit.status, MessageState::Done);
    assert_eq!(actors.status(&deposit.id).await.unwrap().unwrap().message.caused_by, Some(receipt.id.clone()));
    assert!(actors.chain(&MessageId::random()).await.unwrap().is_none());
}

#[tokio::test]
async fn a_typed_send_carries_its_retention_policy_served_with_the_message() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let policy = Retention {
        done: Some(Keep::last(5).unwrap()),
        failed: Some(Keep::keep_for(Duration::from_secs(30 * 24 * 3600)).unwrap()),
    };
    let receipt =
        actors.send_with(&acme(), AccountMsg::Deposit { amount: 1 }, hour().retention(policy.clone())).await.unwrap();
    assert_eq!(actors.status(&receipt.id).await.unwrap().unwrap().message.retention, Some(policy));
    let removed = actors
        .send_with(&acme(), AccountMsg::Explode, SendOptions::new().retention(Retention::REMOVE_ON_FAILED))
        .await
        .unwrap();
    actors.cancel(&receipt.id).await.unwrap();
    eventually(async || actors.status(&removed.id).await.unwrap().is_none()).await;
    let chain = actors.chain(&removed.id).await.unwrap().unwrap();
    assert_eq!(chain.nodes.len(), 1);
    assert!(chain.nodes[0].removed);
}

#[tokio::test]
async fn stateless_types_and_percent_encoded_parameters() {
    let (actors, _, _) = client_test(|r| r).await;
    let odd = account("a b/c é", 1);
    assert!(actors.create(&odd, &AccountState::default()).await.unwrap().created);
    assert_eq!(actors.get(&odd).await.unwrap().unwrap().params.get("tenant"), Some("a b/c é"));
    assert_eq!(actors.list_actors("actor://account/", None, None).await.unwrap().items, [odd.address().unwrap()]);

    let beacon = Beacon { name: "b".into() };
    assert!(actors.create_stateless(&beacon).await.unwrap().created);
    let blink = actors.send(&beacon, BeaconMsg::Blink).await.unwrap();
    eventually(async || actors.status(&blink.id).await.unwrap().map(|s| s.status) == Some(MessageState::Done)).await;
}

#[tokio::test]
async fn dot_segments_are_refused_rather_than_misaddressed() {
    // URL parsing (WHATWG, the url crate) removes `.` and `..` segments, also escaped as %2E, so this client cannot
    // address such instances; it refuses instead of reaching another one. The server serves them (see cicada-axum).
    let (actors, _, _) = client_test(|r| r).await;
    for tenant in [".", ".."] {
        let r = account(tenant, 1);
        assert!(matches!(actors.create(&r, &AccountState::default()).await, Err(ClientError::Invalid(_))));
        assert!(matches!(actors.send(&r, AccountMsg::Deposit { amount: 1 }).await, Err(ClientError::Invalid(_))));
        assert!(matches!(actors.get(&r).await, Err(ClientError::Invalid(_))));
    }
    assert_eq!(account("..", 1).address().unwrap().to_http_path(), "/actors/account/%2E%2E/1");
    assert!(actors.list_actors("", None, None).await.unwrap().items.is_empty());
}

#[tokio::test]
async fn a_resend_with_the_same_id_is_reported_as_duplicate_and_enqueued_once() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let id = MessageId::random();
    let first = actors.send_with(&acme(), AccountMsg::Deposit { amount: 1 }, hour().id(id.clone())).await.unwrap();
    let second = actors.send_with(&acme(), AccountMsg::Deposit { amount: 1 }, hour().id(id)).await.unwrap();
    assert!(!first.duplicate);
    assert!(second.duplicate);
    assert_eq!(first.deliver_at, second.deliver_at);
    assert_eq!(actors.pending(&acme().address().unwrap(), None, None).await.unwrap().unwrap().items.len(), 1);
}

/// Server middleware that answers the first `POST` while `armed` with a broken body after the API accepted it: the
/// client sees a transport error although the send went through.
fn lose_first_ack(armed: Arc<AtomicBool>) -> impl FnOnce(Router) -> Router {
    move |router| {
        router.layer(middleware::from_fn(move |req: Request, next: Next| {
            let armed = armed.clone();
            async move {
                let lose = req.method() == Method::POST && armed.swap(false, Ordering::SeqCst);
                let response = next.run(req).await;
                if !lose {
                    return response;
                }
                let broken = futures::stream::iter([Err::<String, _>(std::io::Error::other("acknowledgement lost"))]);
                (response.status(), Body::from_stream(broken)).into_response()
            }
        }))
    }
}

#[tokio::test]
async fn a_send_whose_acknowledgement_is_lost_is_retried_with_the_same_id() {
    let armed = Arc::new(AtomicBool::new(true));
    let (actors, base, _) = client_test(lose_first_ack(armed.clone())).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let receipt = actors.send_with(&acme(), AccountMsg::Deposit { amount: 1 }, hour()).await.unwrap();
    assert!(!armed.load(Ordering::SeqCst), "the first acknowledgement was lost");
    assert!(receipt.duplicate, "the first attempt had been accepted");
    let pending = actors.pending(&acme().address().unwrap(), None, None).await.unwrap().unwrap();
    assert_eq!(pending.items.iter().map(|m| &m.id).collect::<Vec<_>>(), [&receipt.id]);

    let once = ActorClient::new(&base).send_attempts(1);
    armed.store(true, Ordering::SeqCst);
    assert!(matches!(once.send(&acme(), AccountMsg::Deposit { amount: 2 }).await, Err(ClientError::Http(_))));
}

#[tokio::test]
async fn a_503_on_send_is_retried() {
    let fail_next = Arc::new(AtomicBool::new(true));
    let flag = fail_next.clone();
    let (actors, base, _) = client_test(move |router| {
        router.layer(middleware::from_fn(move |req: Request, next: Next| {
            let flag = flag.clone();
            async move {
                if req.method() == Method::POST && flag.swap(false, Ordering::SeqCst) {
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                next.run(req).await
            }
        }))
    })
    .await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let receipt = actors.send_with(&acme(), AccountMsg::Deposit { amount: 1 }, hour()).await.unwrap();
    assert!(!receipt.duplicate);
    assert_eq!(actors.pending(&acme().address().unwrap(), None, None).await.unwrap().unwrap().items.len(), 1);

    fail_next.store(true, Ordering::SeqCst);
    let once = ActorClient::new(&base).send_attempts(1);
    let e = once.send(&acme(), AccountMsg::Deposit { amount: 1 }).await.unwrap_err();
    assert_eq!(e.status(), Some(503));
}

#[tokio::test]
async fn errors_carry_the_servers_problem() {
    let (actors, _, _) = client_test(|r| r).await;
    let nope = ActorAddress::parse("actor://nope/x").unwrap();
    assert_problem(actors.create_actor(&nope, None).await, 404, pt::NO_SUCH_TYPE);
    assert_problem(
        actors.create_actor(&acme().address().unwrap(), Some(json!({"balance": "lots"}))).await,
        422,
        pt::INVALID_STATE,
    );
    assert_problem(actors.create(&Beacon { name: "b".into() }, &AccountState::default()).await, 422, pt::INVALID_STATE);
    assert_problem(actors.send(&acme(), AccountMsg::Deposit { amount: 1 }).await, 404, pt::NO_SUCH_INSTANCE);

    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let address = acme().address().unwrap();
    assert_problem(
        actors.send_message(&address, "Withdraw", json!({}), SendOptions::new()).await,
        422,
        pt::UNKNOWN_MESSAGE_TYPE,
    );
    assert_problem(
        actors.send_message(&address, "Deposit", json!({"amount": "ten"}), SendOptions::new()).await,
        422,
        pt::INVALID_PAYLOAD,
    );
    let e = actors.list_actors("", None, Some(0)).await.unwrap_err();
    assert_eq!(e.status(), Some(400));
    assert_eq!(e.problem().unwrap().instance.as_deref(), Some("/actors?limit=0"));
    assert!(e.to_string().starts_with("400 Bad request: limit must be"), "{e}");
}

#[tokio::test]
async fn delivery_times_correlation_id_and_headers() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let far = parse_instant("2999-01-01T00:00:00Z").unwrap();
    let at_receipt = actors
        .send_with(
            &acme(),
            AccountMsg::Deposit { amount: 3 },
            SendOptions::new().at(far).correlation_id("corr").header("trace", "t1"),
        )
        .await
        .unwrap();
    assert_eq!(at_receipt.deliver_at, Some(far));
    let raw = actors
        .send_message(
            &acme().address().unwrap(),
            "Deposit",
            json!({"amount": 2}),
            SendOptions { delivery: Delivery::After(Duration::from_secs(3600)), ..SendOptions::new() },
        )
        .await
        .unwrap();
    let delayed = actors
        .send_with(&acme(), AccountMsg::Deposit { amount: 1 }, SendOptions::new().delay(Duration::from_secs(7200)))
        .await
        .unwrap();
    assert!(delayed.deliver_at.unwrap() > raw.deliver_at.unwrap());

    let address = acme().address().unwrap();
    let page = actors.pending(&address, None, Some(2)).await.unwrap().unwrap();
    assert_eq!(page.items.iter().map(|m| &m.id).collect::<Vec<_>>(), [&raw.id, &delayed.id]);
    let rest = actors.pending(&address, page.next.as_deref(), None).await.unwrap().unwrap();
    let [last] = rest.items.as_slice() else { panic!("one more message: {rest:?}") };
    assert_eq!(last.correlation_id.as_deref(), Some("corr"));
    assert_eq!(last.headers, [("trace".to_owned(), "t1".to_owned())].into());
    assert_eq!(last.message_type, "Deposit");
    assert!(actors.pending(&account("acme", 99).address().unwrap(), None, None).await.unwrap().is_none());

    // Across instances: by URI, each mailbox in delivery order; the cursor carries over.
    actors.create(&account("other", 1), &AccountState::default()).await.unwrap();
    let other = actors.send_with(&account("other", 1), AccountMsg::Deposit { amount: 4 }, hour()).await.unwrap();
    let across = actors.list_pending("account/", None, Some(2)).await.unwrap();
    assert_eq!(across.items.iter().map(|m| &m.id).collect::<Vec<_>>(), [&raw.id, &delayed.id]);
    let next = actors.list_pending("account/", across.next.as_deref(), None).await.unwrap();
    assert_eq!(next.items.iter().map(|m| &m.id).collect::<Vec<_>>(), [&at_receipt.id, &other.id]);
}

#[tokio::test]
async fn cancel_outcomes() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let later = actors.send_with(&acme(), AccountMsg::Deposit { amount: 1 }, hour()).await.unwrap();
    assert_eq!(actors.cancel(&later.id).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(actors.status(&later.id).await.unwrap().unwrap().status, MessageState::Cancelled);
    assert_eq!(actors.cancel(&later.id).await.unwrap(), CancelOutcome::AlreadyFinished);
    assert_eq!(actors.cancel(&MessageId::random()).await.unwrap(), CancelOutcome::NotFound);
    assert!(actors.status(&MessageId::random()).await.unwrap().is_none());

    let gate = MessageId::random().to_string();
    let held = actors.send(&acme(), AccountMsg::Hold { gate: gate.clone() }).await.unwrap();
    eventually(async || actors.status(&held.id).await.unwrap().unwrap().status == MessageState::Running).await;
    assert!(actors.status(&held.id).await.unwrap().unwrap().node.is_some());
    assert_eq!(actors.cancel(&held.id).await.unwrap(), CancelOutcome::Running);
    gates::open(&gate);
    eventually(async || actors.status(&held.id).await.unwrap().unwrap().status == MessageState::Done).await;
}

#[tokio::test]
async fn delete_purges_and_list_pages() {
    let (actors, _, _) = client_test(|r| r).await;
    for i in 1..=3 {
        actors.create(&account("acme", i), &AccountState::default()).await.unwrap();
    }
    actors.create(&account("other", 1), &AccountState::default()).await.unwrap();
    let pending = actors.send_with(&account("acme", 1), AccountMsg::Deposit { amount: 1 }, hour()).await.unwrap();

    let first = actors.list_actors("actor://account/acme/", None, Some(2)).await.unwrap();
    assert_eq!(first.items, [account("acme", 1).address().unwrap(), account("acme", 2).address().unwrap()]);
    let second = actors.list_actors("account/acme/", first.next.as_deref(), Some(2)).await.unwrap();
    assert_eq!(second.items, [account("acme", 3).address().unwrap()]);
    assert!(second.next.is_none());
    assert_eq!(actors.list_actors("", None, None).await.unwrap().items.len(), 4);

    assert!(actors.delete(&account("acme", 1)).await.unwrap());
    assert!(!actors.delete(&account("acme", 1)).await.unwrap());
    assert_eq!(actors.status(&pending.id).await.unwrap().unwrap().status, MessageState::Purged);
}

#[tokio::test]
async fn a_message_sent_with_a_retry_policy_shows_its_policy_failures_and_last_error_in_status_and_pending() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let retry = RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(3600)).unwrap()).unwrap();
    let boom = actors.send_with(&acme(), AccountMsg::Explode, SendOptions::new().retry(retry.clone())).await.unwrap();
    eventually(async || actors.status(&boom.id).await.unwrap().is_some_and(|s| s.failures == 1)).await;
    let status = actors.status(&boom.id).await.unwrap().unwrap();
    assert_eq!(status.status, MessageState::Pending);
    let last_error = status.last_error.clone().unwrap();
    assert!(last_error.contains("boom"), "{last_error}");
    assert!(status.error.is_none());
    assert_eq!(status.message.failures, 1);
    assert_eq!(status.message.last_error, Some(last_error));
    assert_eq!(status.message.retry.as_ref(), Some(&retry));
    let page = actors.pending(&acme().address().unwrap(), None, None).await.unwrap().unwrap();
    let [listed] = page.items.as_slice() else { panic!("one pending message: {page:?}") };
    assert_eq!(listed.id, boom.id);
    assert_eq!(listed.failures, 1);
    assert_eq!(listed.retry.as_ref(), Some(&retry));
    assert!(actors.failures("", None, None, None).await.unwrap().items.is_empty());
    assert_eq!(actors.cancel(&boom.id).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(actors.status(&boom.id).await.unwrap().unwrap().failures, 1);
}

#[tokio::test]
async fn send_bodies_leave_out_absent_optional_fields_so_nodes_that_do_not_know_them_yet_accept_them() {
    // A node of an older version decodes request bodies strictly (unknown keys are 400), so a body must not carry
    // "retry": null, or any other absent field, when the caller did not set it.
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorded = bodies.clone();
    let (actors, _, _) = client_test(move |router| {
        router.layer(middleware::from_fn(move |req: Request, next: Next| {
            let recorded = recorded.clone();
            async move {
                let (parts, body) = req.into_parts();
                let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
                recorded.lock().unwrap().push(String::from_utf8(bytes.to_vec()).unwrap());
                next.run(Request::from_parts(parts, Body::from(bytes))).await
            }
        }))
    })
    .await;
    let last = || bodies.lock().unwrap().last().cloned().unwrap();
    let keys = |body: &str| {
        serde_json::from_str::<Value>(body).unwrap().as_object().unwrap().keys().cloned().collect::<Vec<_>>()
    };
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    actors.send(&acme(), AccountMsg::Deposit { amount: 1 }).await.unwrap();
    assert_eq!(keys(&last()), ["id", "type", "payload", "headers"], "{}", last());
    assert!(!last().contains("null"), "{}", last());

    let retry = RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(3600)).unwrap()).unwrap();
    actors
        .send_with(&acme(), AccountMsg::Deposit { amount: 2 }, hour().correlation_id("c").retry(retry))
        .await
        .unwrap();
    assert_eq!(keys(&last()), ["id", "type", "payload", "delay", "correlationId", "headers", "retry"]);
    // Null values inside the payload are data and stay.
    actors
        .send_message(&acme().address().unwrap(), "Deposit", json!({"amount": 3, "note": null}), SendOptions::new())
        .await
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&last()).unwrap()["payload"]["note"], Value::Null);
    // A create without state sends an empty object.
    actors.create_stateless(&Beacon { name: "x".into() }).await.unwrap();
    assert_eq!(last(), "{}");
}

#[tokio::test]
async fn failures_types_and_type_of() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    actors.create(&account("other", 1), &AccountState::default()).await.unwrap();
    let boom = actors.send(&acme(), AccountMsg::Explode).await.unwrap();
    actors.send(&account("other", 1), AccountMsg::Explode).await.unwrap();
    eventually(async || actors.failures("", None, None, None).await.unwrap().items.len() == 2).await;

    let mine = actors.failures("actor://account/acme/", None, None, None).await.unwrap();
    let [mine] = mine.items.as_slice() else { panic!("one failure: {mine:?}") };
    assert_eq!(mine.id, boom.id);
    assert_eq!(mine.status, MessageState::Failed);
    assert!(mine.error.as_deref().unwrap().contains("boom"));
    assert_eq!(mine.message.message_type, "Explode");
    let far = parse_instant("2999-01-01T00:00:00Z").unwrap();
    assert!(actors.failures("", Some(far), None, None).await.unwrap().items.is_empty());
    assert_eq!(actors.failures("", None, None, Some(1)).await.unwrap().items.len(), 1);

    let types = actors.types().await.unwrap();
    assert_eq!(
        types.iter().map(|t| t.template.as_str()).collect::<Vec<_>>(),
        ["actor://account/{tenant}/{id}", "actor://beacon/{name}"]
    );
    assert_eq!(
        types[0].messages.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Deposit", "Explode", "Hold", "Transfer"]
    );
    assert!(types[1].state.is_none());

    let found = actors.type_of(&acme().address().unwrap()).await.unwrap().unwrap();
    assert_eq!(found.params.iter().collect::<Vec<_>>(), vec![("tenant", "acme"), ("id", "7")]);
    assert_eq!(found.actor_type, types[0]);
    assert!(actors.type_of(&ActorAddress::parse("actor://nope/x").unwrap()).await.unwrap().is_none());
}

#[tokio::test]
async fn health_follows_the_node() {
    let (actors, _, system) = client_test(|r| r).await;
    assert!(actors.health().await.unwrap());
    system.stop().await;
    assert!(!actors.health().await.unwrap());
}

#[tokio::test]
async fn the_clients_own_configuration_authenticates_and_non_api_errors_are_errors_too() {
    const BASIC: &str = "Basic c3ZjOnNlY3JldA=="; // svc:secret
    let system = node_with(|b| b.host(AccountActor)).await;
    let app = actor_api(system).layer(middleware::from_fn(|req: Request, next: Next| async move {
        if req.headers().get(header::AUTHORIZATION).is_some_and(|v| v == BASIC) {
            next.run(req).await
        } else {
            (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Basic")]).into_response()
        }
    }));
    let base = serve(app).await;
    let authed = reqwest::Client::builder()
        .default_headers([(header::AUTHORIZATION, HeaderValue::from_static(BASIC))].into_iter().collect())
        .build()
        .unwrap();
    let actors = ActorClient::with_client(&base, authed.clone());
    assert!(actors.health().await.unwrap());
    assert!(actors.create(&acme(), &AccountState::default()).await.unwrap().created);

    let anonymous = ActorClient::new(&base);
    let e = anonymous.health().await.unwrap_err();
    assert_eq!(e.status(), Some(401));
    assert_eq!(e.problem().unwrap().problem_type, "about:blank");
    assert_eq!(e.problem().unwrap().title, "Unauthorized");

    // A 404 that is not the API's own (wrong base URL) is not mistaken for "absent".
    let wrong_base = ActorClient::with_client(format!("{base}/wrong"), authed);
    assert_eq!(wrong_base.get(&acme()).await.unwrap_err().status(), Some(404));
}

#[test]
#[should_panic(expected = "send_attempts must be at least 1")]
fn zero_send_attempts_are_refused() {
    let _ = ActorClient::new("http://localhost").send_attempts(0);
}

#[test]
fn errors_read_as_status_title_and_detail() {
    let e = ClientError::Api(cicada::api::Problem {
        problem_type: pt::NO_SUCH_TYPE.into(),
        title: "No such actor type".into(),
        status: 404,
        detail: Some("No registered actor type matches actor://nope/x".into()),
        instance: None,
    });
    assert_eq!(e.to_string(), "404 No such actor type: No registered actor type matches actor://nope/x");
}
