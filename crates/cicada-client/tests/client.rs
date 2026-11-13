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
use cicada::address::TypeName;
use cicada::api::{BulkRequest, MessageState, problem_types as pt};
use cicada::prelude::*;
use cicada::store::{Delivery, MessageId, NodeId, OutcomeStatus, RemoveTypeResult};
use cicada::time::parse_instant;
use cicada::{ActorAddress, WatchTarget};
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
async fn watches_are_set_listed_and_removed() {
    let system = node_with(|b| b.host(AccountActor).host(WatcherActor)).await;
    let actors = ActorClient::new(serve(actor_api(system)).await);
    let watcher = Watcher { id: "w".into() };
    actors.create(&watcher, &NoState {}).await.unwrap();
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let (watcher, target) = (watcher.address().unwrap(), WatchTarget::Actor(acme().address().unwrap()));

    assert!(actors.watch(&target, &watcher).await.unwrap(), "new");
    assert!(!actors.watch(&WatchTarget::Actor(acme().address().unwrap()), &watcher).await.unwrap(), "set already");
    assert!(actors.watch(&WatchTarget::Every, &watcher).await.unwrap());
    let all = actors.watches(None, None, None, None).await.unwrap();
    let listed: Vec<String> = all.items.iter().map(|w| format!("{} {}", w.target, w.watcher)).collect();
    assert_eq!(listed, ["* watcher/w", "account/acme:7 watcher/w"]);
    let page = actors.watches(Some(&watcher), Some(&target), None, Some(1)).await.unwrap();
    assert_eq!((page.items.len(), page.next), (1, None));

    let account = acme().address().unwrap();
    assert_problem(actors.watch(&target, &account).await, 422, pt::NOT_A_WATCHER);
    let absent = ActorAddress::parse("watcher/nosuch").unwrap();
    assert_problem(actors.watch(&target, &absent).await, 404, pt::NO_SUCH_INSTANCE);
    assert!(actors.unwatch(&target, &watcher).await.unwrap());
    assert!(!actors.unwatch(&target, &watcher).await.unwrap(), "not stored any more");
    assert_eq!(actors.watches(None, None, None, None).await.unwrap().items.len(), 1);
}

#[tokio::test]
async fn typed_create_send_state_and_status() {
    let (actors, _, _) = client_test(|r| r).await;
    assert!(actors.create(&acme(), &owned_by("Ada")).await.unwrap().created);
    let again = actors.create(&acme(), &owned_by("Bob")).await.unwrap();
    assert!(!again.created);
    assert_eq!(again.address, acme().address().unwrap());

    let receipt = actors.send(&acme(), AccountMsg::Deposit { amount: 5 }).await.unwrap();
    assert!(!receipt.duplicate);
    assert!(receipt.deliver_at.is_some());
    eventually(async || actors.status(&receipt.id).await.unwrap().map(|s| s.status) == Some(MessageState::Done)).await;
    assert_eq!(actors.state(&acme()).await.unwrap(), Some(AccountState { balance: 5, owner: "Ada".into() }));

    let view = actors.get(&acme()).await.unwrap().unwrap();
    assert_eq!(view.address, acme().address().unwrap());
    assert_eq!(view.type_name, "account");
    assert_eq!(view.address.as_str(), "account/acme:7");
    assert_eq!(view.version, 1);

    assert!(actors.get(&account("acme", 8)).await.unwrap().is_none());
    assert!(actors.state::<_, AccountState>(&account("acme", 8)).await.unwrap().is_none());
}

#[tokio::test]
async fn the_cause_of_a_message_is_in_its_envelope() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    actors.create(&account("acme", 8), &AccountState::default()).await.unwrap();
    // The receiver is busy, so the deposit stays in its mailbox.
    let gate = MessageId::random().to_string();
    let held = actors.send(&account("acme", 8), AccountMsg::Hold { gate: gate.clone() }).await.unwrap();
    eventually(async || actors.status(&held.id).await.unwrap().unwrap().status == MessageState::Running).await;
    let receipt = actors.send(&acme(), AccountMsg::Transfer { to: 8, amount: 3 }).await.unwrap();
    eventually(async || actors.status(&receipt.id).await.unwrap().map(|s| s.status) == Some(MessageState::Done)).await;

    assert_eq!(actors.status(&receipt.id).await.unwrap().unwrap().message.caused_by, None);
    let mailbox = actors.mailbox(&account("acme", 8).address().unwrap(), None, None).await.unwrap().unwrap();
    let deposits: Vec<_> = mailbox.items.iter().filter(|m| m.message.message_type == "Deposit").collect();
    let [deposit] = deposits.as_slice() else { panic!("one deposit: {mailbox:?}") };
    assert_eq!(deposit.message.caused_by.as_ref(), Some(&receipt.id));
    assert_eq!(actors.status(&deposit.message.id).await.unwrap().unwrap().message.caused_by, Some(receipt.id.clone()));

    gates::open(&gate);
    eventually(async || balance(&actors, &account("acme", 8)).await == Some(3)).await;
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
}

#[tokio::test]
async fn types_that_keep_no_data_and_percent_encoded_parameters() {
    let (actors, _, _) = client_test(|r| r).await;
    let odd = account("a b/c é", 1);
    assert!(actors.create(&odd, &AccountState::default()).await.unwrap().created);
    assert_eq!(actors.get(&odd).await.unwrap().unwrap().address.name(), "a b/c é:1");
    assert_eq!(actors.list_actors("account/", None, None).await.unwrap().items, [odd.address().unwrap()]);

    let beacon = Beacon { name: "b".into() };
    assert!(actors.create(&beacon, &NoState {}).await.unwrap().created);
    let blink = actors.send(&beacon, BeaconMsg::Blink).await.unwrap();
    eventually(async || actors.status(&blink.id).await.unwrap().map(|s| s.status) == Some(MessageState::Done)).await;
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
    assert_eq!(actors.mailbox(&acme().address().unwrap(), None, None).await.unwrap().unwrap().items.len(), 1);
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
    let pending = actors.mailbox(&acme().address().unwrap(), None, None).await.unwrap().unwrap();
    assert_eq!(pending.items.iter().map(|m| &m.message.id).collect::<Vec<_>>(), [&receipt.id]);

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
    assert_eq!(actors.mailbox(&acme().address().unwrap(), None, None).await.unwrap().unwrap().items.len(), 1);

    fail_next.store(true, Ordering::SeqCst);
    let once = ActorClient::new(&base).send_attempts(1);
    let e = once.send(&acme(), AccountMsg::Deposit { amount: 1 }).await.unwrap_err();
    assert_eq!(e.status(), Some(503));
}

#[tokio::test]
async fn errors_carry_the_servers_problem() {
    let (actors, _, _) = client_test(|r| r).await;
    let nope = ActorAddress::parse("nope/x").unwrap();
    assert_problem(actors.create_actor(&nope, json!({})).await, 404, pt::NO_SUCH_TYPE);
    assert_problem(
        actors.create_actor(&acme().address().unwrap(), json!({"balance": "lots"})).await,
        422,
        pt::INVALID_STATE,
    );
    assert_problem(actors.create(&Beacon { name: "b".into() }, &AccountState::default()).await, 422, pt::INVALID_STATE);
    assert_problem(actors.send(&acme(), AccountMsg::Deposit { amount: 1 }).await, 404, pt::NO_SUCH_INSTANCE);
    assert_problem(actors.send_message(&nope, "Deposit", json!({}), SendOptions::new()).await, 404, pt::NO_SUCH_TYPE);

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

/// http-schema-check: the refusal of a node that does not host the type reaches the caller with its detail.
#[tokio::test]
async fn a_node_that_does_not_host_the_type_refuses_what_does_not_match_the_schema_with_its_detail() {
    let store = cicada::Store::memory();
    ActorSystem::builder(store.clone()).host(AccountActor).build().initialize().await.unwrap();
    let send_only = ActorSystem::builder(store).build();
    send_only.start().await.unwrap();
    let actors = ActorClient::new(&serve(actor_api(send_only)).await);
    let address = acme().address().unwrap();

    let refused = actors.create_actor(&address, json!({"balance": "lots"})).await.unwrap_err();
    let problem = refused.problem().expect("a problem");
    assert_eq!((problem.status, problem.problem_type.as_str()), (422, pt::INVALID_STATE));
    let detail = "State does not match the registered schema; GET /types/account has it";
    assert_eq!(problem.detail.as_deref(), Some(detail));

    actors.create(&acme(), &AccountState::default()).await.unwrap();
    let refused = actors.send_message(&address, "Deposit", json!({"amount": "ten"}), SendOptions::new()).await;
    let problem = refused.unwrap_err().problem().cloned().expect("a problem");
    assert_eq!((problem.status, problem.problem_type.as_str()), (422, pt::INVALID_PAYLOAD));
    let detail = "Payload of 'Deposit' does not match the registered schema; GET /types/account has it";
    assert_eq!(problem.detail.as_deref(), Some(detail));
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
    let page = actors.mailbox(&address, None, Some(2)).await.unwrap().unwrap();
    assert_eq!(page.items.iter().map(|m| &m.message.id).collect::<Vec<_>>(), [&raw.id, &delayed.id]);
    let rest = actors.mailbox(&address, page.next.as_deref(), None).await.unwrap().unwrap();
    let [last] = rest.items.as_slice() else { panic!("one more message: {rest:?}") };
    assert_eq!(last.message.correlation_id.as_deref(), Some("corr"));
    assert_eq!(last.message.headers, [("trace".to_owned(), "t1".to_owned())].into());
    assert_eq!(last.message.message_type, "Deposit");
    assert!(actors.mailbox(&account("acme", 99).address().unwrap(), None, None).await.unwrap().is_none());

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

    let first = actors.list_actors("account/acme:", None, Some(2)).await.unwrap();
    assert_eq!(first.items, [account("acme", 1).address().unwrap(), account("acme", 2).address().unwrap()]);
    let second = actors.list_actors("account/acme:", first.next.as_deref(), Some(2)).await.unwrap();
    assert_eq!(second.items, [account("acme", 3).address().unwrap()]);
    assert!(second.next.is_none());
    assert_eq!(actors.list_actors("", None, None).await.unwrap().items.len(), 4);

    assert!(actors.delete(&account("acme", 1)).await.unwrap());
    assert!(!actors.delete(&account("acme", 1)).await.unwrap());
    assert_eq!(actors.status(&pending.id).await.unwrap().unwrap().status, MessageState::Purged);
}

/// Scenarios: A page holds complete stored snapshots; Pages have the same boundaries as address-only pages.
#[tokio::test]
async fn actors_are_listed_with_their_state_in_the_pages_of_the_address_listing() {
    let (actors, _, _) = client_test(|r| r).await;
    for i in 1..=3 {
        actors.create(&account("acme", i), &AccountState::default()).await.unwrap();
    }
    actors.create(&account("other", 1), &AccountState::default()).await.unwrap();
    actors.send(&account("acme", 2), AccountMsg::Deposit { amount: 5 }).await.unwrap();
    eventually(async || balance(&actors, &account("acme", 2)).await == Some(5)).await;
    assert!(actors.pause_mailbox(&account("acme", 3).address().unwrap()).await.unwrap());

    let first = actors.list_actors_with_state("account/acme:", None, Some(2)).await.unwrap();
    let addresses = actors.list_actors("account/acme:", None, Some(2)).await.unwrap();
    assert_eq!(first.items.iter().map(|v| v.address.clone()).collect::<Vec<_>>(), addresses.items);
    assert_eq!(first.next, addresses.next);
    let second = actors.list_actors_with_state("account/acme:", first.next.as_deref(), Some(2)).await.unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(second.next.is_none());
    for view in first.items.iter().chain(&second.items) {
        assert_eq!(Some(view), actors.get_actor(&view.address).await.unwrap().as_ref());
    }
    let [_, deposited] = &first.items[..] else { panic!("two views") };
    assert_eq!(
        (deposited.state["balance"].clone(), deposited.version, deposited.type_name.as_str()),
        (json!(5), 1, "account")
    );
    assert!(second.items[0].paused);
    assert_eq!(actors.list_actors_with_state("", None, None).await.unwrap().items.len(), 4);
    assert!(actors.list_actors_with_state("nobody/", None, None).await.unwrap().items.is_empty());
    let e = actors.list_actors_with_state("", None, Some(0)).await.unwrap_err();
    assert_eq!(e.status(), Some(400));
}

/// Scenario: The generated contract and client describe the new route. Against a server without `GET /actors/states`
/// (one before 0.16.9), the method returns that server's error after one request and reads no actor one by one.
#[tokio::test]
async fn the_state_listing_against_an_older_server_returns_its_error_and_does_not_fall_back() {
    let paths = Arc::new(Mutex::new(Vec::new()));
    let seen = paths.clone();
    let older = move |router: Router| {
        router.layer(middleware::from_fn(move |req: Request, next: Next| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(req.uri().path().to_owned());
                if req.uri().path() == "/actors/states" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                next.run(req).await
            }
        }))
    };
    let (actors, _, _) = client_test(older).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    paths.lock().unwrap().clear();
    let e = actors.list_actors_with_state("", None, None).await.unwrap_err();
    assert_eq!(e.status(), Some(404));
    assert_eq!(*paths.lock().unwrap(), ["/actors/states"]);
}

#[tokio::test]
async fn the_children_of_an_actor_are_listed_its_children_name_it_and_a_delete_removes_them() {
    let (actors, _, _) = client_test(|r| r).await;
    let parent = account("acme", 1);
    let address = parent.address().unwrap();
    actors.create(&parent, &AccountState::default()).await.unwrap();
    for id in [3, 2] {
        actors.send(&parent, AccountMsg::Open { id }).await.unwrap();
    }
    eventually(async || actors.children(&address, None, None).await.unwrap().unwrap().items.len() == 2).await;

    let first = actors.children(&address, None, Some(1)).await.unwrap().expect("the parent exists");
    assert_eq!(first.items, [account("acme", 2).address().unwrap()]);
    let second = actors.children(&address, first.next.as_deref(), Some(1)).await.unwrap().unwrap();
    assert_eq!(second.items, [account("acme", 3).address().unwrap()]);
    assert!(second.next.is_none());
    assert_eq!(actors.get(&account("acme", 2)).await.unwrap().unwrap().parent, Some(address.clone()));
    assert_eq!(actors.get(&parent).await.unwrap().unwrap().parent, None);
    assert_eq!(actors.children(&account("nobody", 1).address().unwrap(), None, None).await.unwrap(), None);

    assert!(actors.delete(&parent).await.unwrap());
    assert_eq!(actors.get(&account("acme", 2)).await.unwrap(), None, "the delete removed the children too");
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
    // A create of a type that keeps no data sends its empty state.
    actors.create(&Beacon { name: "x".into() }, &NoState {}).await.unwrap();
    assert_eq!(last(), r#"{"state":{}}"#);
}

#[tokio::test]
async fn outcomes_and_types() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    actors.create(&account("other", 1), &AccountState::default()).await.unwrap();
    let boom = actors.send(&acme(), AccountMsg::Explode).await.unwrap();
    actors.send(&account("other", 1), AccountMsg::Explode).await.unwrap();
    eventually(async || {
        actors.outcomes(Some(OutcomeStatus::Failed), "", None, None, None).await.unwrap().items.len() == 2
    })
    .await;

    let mine = actors.outcomes(Some(OutcomeStatus::Failed), "account/acme:", None, None, None).await.unwrap();
    let [mine] = mine.items.as_slice() else { panic!("one failure: {mine:?}") };
    assert_eq!(mine.id, boom.id);
    assert_eq!(mine.status, MessageState::Failed);
    assert!(mine.error.as_deref().unwrap().contains("boom"));
    assert_eq!(mine.message.message_type, "Explode");
    let far = parse_instant("2999-01-01T00:00:00Z").unwrap();
    assert!(actors.outcomes(Some(OutcomeStatus::Failed), "", Some(far), None, None).await.unwrap().items.is_empty());
    assert_eq!(actors.outcomes(Some(OutcomeStatus::Failed), "", None, None, Some(1)).await.unwrap().items.len(), 1);

    // A finished message of another status shows with `status=done` and without a status, not under `failed`.
    let deposit = actors.send(&acme(), AccountMsg::Deposit { amount: 1 }).await.unwrap();
    eventually(async || {
        let done = actors.outcomes(Some(OutcomeStatus::Done), "account/acme:", None, None, None).await.unwrap();
        done.items.iter().any(|o| o.id == deposit.id)
    })
    .await;
    let everything = actors.outcomes(None, "", None, None, None).await.unwrap();
    assert!(everything.items.iter().any(|o| o.id == deposit.id) && everything.items.iter().any(|o| o.id == boom.id));
    assert_eq!(actors.outcomes(Some(OutcomeStatus::Failed), "", None, None, None).await.unwrap().items.len(), 2);

    let types = actors.types().await.unwrap();
    assert_eq!(types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["account", "beacon"]);
    assert_eq!(
        types[0].messages.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Deposit", "Explode", "Hold", "Open", "Transfer"]
    );
    assert_eq!(types[1].state["type"], "object");
}

#[tokio::test]
async fn caused_by_lists_what_a_message_sent() {
    let (actors, _, _) = client_test(|r| r).await;
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    actors.create(&account("acme", 2), &AccountState::default()).await.unwrap();
    let transfer = actors.send(&acme(), AccountMsg::Transfer { to: 2, amount: 3 }).await.unwrap();
    let caused = async || actors.caused_by(&transfer.id, None, None).await.unwrap().unwrap();
    eventually(async || {
        caused().await.items.iter().all(|c| c.status == MessageState::Done) && !caused().await.items.is_empty()
    })
    .await;

    let page = caused().await;
    let [deposit] = page.items.as_slice() else { panic!("one caused message: {page:?}") };
    assert_eq!(deposit.message.message_type, "Deposit");
    assert_eq!(deposit.message.caused_by, Some(transfer.id.clone()));
    assert_eq!(page.next, None);
    assert!(actors.caused_by(&deposit.id, None, Some(5)).await.unwrap().unwrap().items.is_empty());
    assert!(actors.caused_by(&MessageId::random(), None, None).await.unwrap().is_none());
}

#[tokio::test]
async fn stats_count_what_is_stored() {
    let (actors, _, _) = client_test(|r| r).await;
    let none = actors.stats("").await.unwrap();
    assert_eq!((none.actors, none.pending, none.oldest_due), (0, 0, None));
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    actors.create(&account("other", 1), &AccountState::default()).await.unwrap();
    actors.send_with(&acme(), AccountMsg::Deposit { amount: 1 }, hour()).await.unwrap();
    let all = actors.stats("").await.unwrap();
    assert_eq!((all.actors, all.pending, all.oldest_due), (2, 1, None), "the message is delayed");
    let mine = actors.stats("account/acme:").await.unwrap();
    assert_eq!((mine.actors, mine.pending), (1, 1));
    assert_eq!(actors.stats("beacon/").await.unwrap().actors, 0);
}

#[tokio::test]
async fn remove_type_answers_removed_absent_and_in_use() {
    let (actors, _, _) = client_test(|r| r).await;
    let beacon = TypeName::parse("beacon").unwrap();
    actors.create(&acme(), &AccountState::default()).await.unwrap();
    assert_eq!(actors.remove_type(&TypeName::parse("account").unwrap()).await.unwrap(), RemoveTypeResult::InUse);
    assert_eq!(actors.remove_type(&beacon).await.unwrap(), RemoveTypeResult::Removed);
    assert_eq!(actors.remove_type(&beacon).await.unwrap(), RemoveTypeResult::Absent);
    assert!(actors.get_type(&beacon).await.unwrap().is_none());
    assert!(actors.delete(&acme()).await.unwrap());
    assert_eq!(actors.remove_type(&TypeName::parse("account").unwrap()).await.unwrap(), RemoveTypeResult::Removed);
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

#[tokio::test]
async fn nodes_turns_mailbox_actions_retry_and_bulk() {
    let (actors, _, system) = client_test(|r| r).await;
    let acme_address = acme().address().unwrap();
    actors.create(&acme(), &AccountState::default()).await.unwrap();

    // Nodes and running turns.
    let node_id = system.node_id().clone();
    eventually(async || !actors.nodes().await.unwrap().is_empty()).await;
    assert_eq!(actors.nodes().await.unwrap()[0].id, node_id);
    assert!(actors.node(&NodeId::new("nope").unwrap()).await.unwrap().is_none());

    let gate = MessageId::random().to_string();
    let held = actors.send(&acme(), AccountMsg::Hold { gate: gate.clone() }).await.unwrap();
    eventually(async || actors.status(&held.id).await.unwrap().unwrap().status == MessageState::Running).await;
    assert_eq!(actors.node(&node_id).await.unwrap().unwrap().turns[0].message_id, held.id);
    assert_eq!(actors.turns(Some(&node_id), "", None, None).await.unwrap().items.len(), 1);
    assert_eq!(actors.turn(&acme_address).await.unwrap().unwrap().message_id, held.id);

    // The mailbox marks the running message; pause, purge and resume.
    let queued = actors.send(&acme(), AccountMsg::Deposit { amount: 1 }).await.unwrap();
    let mailbox = actors.mailbox(&acme_address, None, None).await.unwrap().unwrap();
    assert_eq!(mailbox.items[0].state, MessageState::Running);
    assert_eq!(mailbox.items[1].state, MessageState::Pending);
    assert!(actors.pause_mailbox(&acme_address).await.unwrap());
    assert!(actors.get(&acme()).await.unwrap().unwrap().paused);
    assert_eq!(actors.purge_mailbox(&acme_address).await.unwrap(), Some(1));
    assert_eq!(actors.status(&queued.id).await.unwrap().unwrap().status, MessageState::Purged);
    assert!(actors.resume_mailbox(&acme_address).await.unwrap());

    // Cancelling the running turn takes effect at the next heartbeat.
    assert!(actors.cancel_turn(&acme_address).await.unwrap());
    eventually(async || actors.status(&held.id).await.unwrap().unwrap().status == MessageState::Cancelled).await;
    assert!(actors.turn(&acme_address).await.unwrap().is_none());
    assert!(!actors.cancel_turn(&acme_address).await.unwrap());
    gates::open(&gate);

    // Retrying the cancelled message; deliver now; bulk.
    let again = actors.retry(&held.id).await.unwrap().unwrap();
    assert_ne!(again.id, held.id);
    assert!(actors.retry(&MessageId::random()).await.unwrap().is_none());
    assert_problem(actors.retry(&queued.id).await, 409, pt::MESSAGE_NOT_RETRYABLE);

    let later = actors.send_with(&acme(), AccountMsg::Deposit { amount: 2 }, hour()).await.unwrap();
    assert_eq!(actors.deliver_now(&later.id).await.unwrap().unwrap().id, later.id);
    eventually(async || actors.status(&later.id).await.unwrap().unwrap().status == MessageState::Done).await;
    assert!(actors.deliver_now(&MessageId::random()).await.unwrap().is_none());

    let delayed = actors.send_with(&acme(), AccountMsg::Deposit { amount: 3 }, hour()).await.unwrap();
    let result = actors.bulk_cancel(&BulkRequest::ids(vec![delayed.id.clone(), held.id.clone()])).await.unwrap();
    assert_eq!((result.affected, result.more), (1, false));
    assert_eq!(actors.bulk_retry(&BulkRequest::ids(vec![delayed.id.clone()])).await.unwrap().affected, 1);

    assert_problem(
        actors.bulk_cancel(&BulkRequest { ids: Some(vec![]), prefix: Some("a".into()), message_type: None }).await,
        400,
        pt::BAD_REQUEST,
    );
}

#[test]
fn errors_read_as_status_title_and_detail() {
    let e = ClientError::Api(cicada::api::Problem {
        problem_type: pt::NO_SUCH_TYPE.into(),
        title: "No such actor type".into(),
        status: 404,
        detail: Some("No registered actor type matches nope/x".into()),
        instance: None,
    });
    assert_eq!(e.to_string(), "404 No such actor type: No registered actor type matches nope/x");
}

/// Every route the client calls is in the OpenAPI document of the server.
#[test]
fn the_client_only_calls_documented_routes() {
    let document = cicada_axum::openapi_document();
    let used = [
        ("put", "/actors/{type}/{key}"),
        ("get", "/actors/{type}/{key}"),
        ("delete", "/actors/{type}/{key}"),
        ("get", "/actors"),
        ("post", "/actors/{type}/{key}/messages"),
        ("get", "/actors/{type}/{key}/mailbox"),
        ("post", "/actors/{type}/{key}/mailbox/pause"),
        ("post", "/actors/{type}/{key}/mailbox/resume"),
        ("post", "/actors/{type}/{key}/mailbox/purge"),
        ("get", "/actors/{type}/{key}/turn"),
        ("post", "/actors/{type}/{key}/turn/cancel"),
        ("get", "/messages"),
        ("get", "/messages/{id}"),
        ("get", "/messages/{id}/caused"),
        ("post", "/messages/{id}/cancel"),
        ("post", "/messages/{id}/retry"),
        ("post", "/messages/{id}/deliver-now"),
        ("post", "/messages/bulk-cancel"),
        ("post", "/messages/bulk-retry"),
        ("get", "/nodes"),
        ("get", "/nodes/{node}"),
        ("get", "/turns"),
        ("get", "/outcomes"),
        ("get", "/stats"),
        ("get", "/types"),
        ("get", "/types/{name}"),
        ("delete", "/types/{name}"),
        ("get", "/health"),
    ];
    for (method, path) in used {
        assert!(document["paths"][path][method].is_object(), "{method} {path}");
    }
}
