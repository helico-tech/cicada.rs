//! The routes of `/watches` (ADR 0023): set, remove and list watches over HTTP, their refusals, and what a watcher is
//! told when its target is deleted or a message to it fails.

mod common;

use axum::Router;
use axum::http::StatusCode;
use cicada::api::problem_types as pt;
use cicada::prelude::*;
use cicada_axum::actor_api;
use common::*;
use serde_json::{Value, json};

/// A node with the counter and watcher types, its API, and the watcher `watcher/a` and the counter `counter/c`.
async fn app() -> (Router, ActorSystem) {
    let system = node_with(memory_store(), |b| b.host(CounterActor).host(WatcherActor)).await;
    system.create(&Watcher { id: "a".into() }, &Seen::default()).await.unwrap();
    system.create(&Counter { id: "c".into() }, &CounterState::default()).await.unwrap();
    (actor_api(system.clone()), system)
}

fn body(target: &str, watcher: &str) -> String {
    json!({"target": target, "watcher": watcher}).to_string()
}

async fn listed(app: &Router, query: &str) -> Vec<Value> {
    items(&get_json(app, &format!("/watches{query}")).await).clone()
}

fn pair(target: &str, watcher: &str) -> Value {
    json!({"target": target, "watcher": watcher})
}

async fn seen(system: &ActorSystem, key: &str) -> Seen {
    let snapshot = system.get_actor(&Watcher { id: key.into() }.address().unwrap()).await.unwrap().unwrap();
    serde_json::from_value(snapshot.state).unwrap()
}

/// Scenario: Setting and removing over HTTP.
#[tokio::test(start_paused = true)]
async fn setting_and_removing_over_http() {
    let (app, _) = app().await;
    let set = put(&app, "/watches", &body("counter/c", "watcher/a")).await;
    assert_eq!(set.status, StatusCode::CREATED, "{}", set.text);
    assert_eq!(set.json(), pair("counter/c", "watcher/a"));
    let again = put(&app, "/watches", &body("counter/c", "watcher/a")).await;
    assert_eq!(again.status, StatusCode::OK, "idempotent: {}", again.text);
    assert_eq!(listed(&app, "").await, [pair("counter/c", "watcher/a")], "the list holds the watch once");
    let removed = delete(&app, &format!("/watches?target={}&watcher={}", enc("counter/c"), enc("watcher/a"))).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.text);
    assert!(listed(&app, "").await.is_empty());
}

/// Scenario: Removing a watch that does not exist.
#[tokio::test(start_paused = true)]
async fn removing_a_watch_that_does_not_exist_is_404_no_such_watch() {
    let (app, _) = app().await;
    let reply = delete(&app, &format!("/watches?target=billing&watcher={}", enc("watcher/a"))).await;
    assert_problem(&reply, 404, pt::NO_SUCH_WATCH);
    assert_problem(&delete(&app, "/watches?target=billing").await, 400, pt::BAD_REQUEST);
    assert_problem(
        &delete(&app, &format!("/watches?target=a%2F%2Fb&watcher={}", enc("watcher/a"))).await,
        400,
        pt::BAD_REQUEST,
    );
}

/// Scenarios: A watcher that does not exist; A watcher whose type is not registered; A watcher whose type lacks a
/// message; A text that is no target (over HTTP).
#[tokio::test(start_paused = true)]
async fn a_watch_is_refused_for_a_bad_watcher_or_target() {
    let (app, _) = app().await;
    assert_problem(&put(&app, "/watches", &body("counter/c", "watcher/nosuch")).await, 404, pt::NO_SUCH_INSTANCE);
    assert_problem(&put(&app, "/watches", &body("counter/c", "nosuch/x")).await, 404, pt::NO_SUCH_TYPE);
    let lacks = put(&app, "/watches", &body("watcher/a", "counter/c")).await;
    assert_problem(&lacks, 422, pt::NOT_A_WATCHER);
    assert!(lacks.json()["detail"].as_str().unwrap().contains("has no message type Failed"), "{}", lacks.text);
    for bad in [
        body("billing..x/", "watcher/a"),
        body("counter/c", "not an address"),
        json!({"target": "counter/c", "watcher": "watcher/a", "extra": 1}).to_string(),
        json!({"target": "counter/c"}).to_string(),
        String::new(),
    ] {
        assert_problem(&put(&app, "/watches", &bad).await, 400, pt::BAD_REQUEST);
    }
    assert!(listed(&app, "").await.is_empty(), "no watch is stored");
}

/// Scenario: An actor is deleted (over HTTP).
#[tokio::test(start_paused = true)]
async fn a_watched_actor_deleted_over_http_tells_its_watcher() {
    let (app, system) = app().await;
    assert_eq!(put(&app, "/watches", &body("counter/c", "watcher/a")).await.status, StatusCode::CREATED);
    assert_eq!(delete(&app, "/actors/counter/c").await.status, StatusCode::NO_CONTENT);
    eventually(async || !seen(&system, "a").await.stopped.is_empty()).await;
    let stopped = seen(&system, "a").await.stopped;
    assert_eq!(
        serde_json::to_value(&stopped).unwrap(),
        json!([{"actor": "counter/c", "cause": "deleted", "by": null}])
    );
    // The view of the notification says that it is one.
    let outcomes = get_json(&app, &format!("/outcomes?prefix={}", enc("watcher/a"))).await;
    let message = &items(&outcomes)[0]["message"];
    assert_eq!((message["type"].clone(), message["notification"].clone()), (json!("Stopped"), json!(true)));
}

/// Scenarios: The watches of one watcher; The watches of one target; Both filters, and a bad one.
#[tokio::test(start_paused = true)]
async fn the_watches_are_listed_by_watcher_and_by_exact_target_with_paging() {
    let (app, system) = app().await;
    system.create(&Watcher { id: "b".into() }, &Seen::default()).await.unwrap();
    for (target, watcher) in
        [("billing", "watcher/a"), ("billing.invoices/acme", "watcher/a"), ("billing", "watcher/b"), ("*", "watcher/b")]
    {
        assert_eq!(put(&app, "/watches", &body(target, watcher)).await.status, StatusCode::CREATED);
    }
    let of_a = listed(&app, &format!("?watcher={}", enc("watcher/a"))).await;
    assert_eq!(of_a, [pair("billing", "watcher/a"), pair("billing.invoices/acme", "watcher/a")]);
    let of_billing = listed(&app, "?target=billing").await;
    assert_eq!(of_billing, [pair("billing", "watcher/a"), pair("billing", "watcher/b")], "exactly that target");
    assert_eq!(
        listed(&app, &format!("?target=billing&watcher={}", enc("watcher/b"))).await,
        [pair("billing", "watcher/b")]
    );
    assert!(listed(&app, &format!("?target=%2A&watcher={}", enc("watcher/a"))).await.is_empty(), "not stored");
    assert_problem(&get(&app, "/watches?target=a%2F%2Fb").await, 400, pt::BAD_REQUEST);
    assert_problem(&get(&app, "/watches?watcher=nope").await, 400, pt::BAD_REQUEST);

    let first = get_json(&app, "/watches?limit=3").await;
    assert_eq!(items(&first).len(), 3);
    let next = first["next"].as_str().expect("a next page");
    let second = get_json(&app, &format!("/watches?limit=3&cursor={}", enc(next))).await;
    assert_eq!(items(&second), &[pair("billing.invoices/acme", "watcher/a")]);
    assert_eq!(second["next"], Value::Null);
}

/// Scenario: A sender cannot mark a message (over HTTP).
#[tokio::test(start_paused = true)]
async fn a_message_of_the_type_failed_sent_over_http_is_not_marked() {
    let (app, system) = app().await;
    system.create(&Watcher { id: "refusing".into() }, &Seen { refuse: true, ..Seen::default() }).await.unwrap();
    assert_eq!(put(&app, "/watches", &body("watcher/refusing", "watcher/a")).await.status, StatusCode::CREATED);
    let fake = json!({"type": "Failed", "payload": {"actor": "counter/c", "messageId": "m1", "messageType": "Add",
        "payload": {}, "correlationId": null, "from": null, "error": "made up", "attempts": 1}});
    let id = send_id(&app, "/actors/watcher/refusing/messages", &fake.to_string()).await;
    assert_eq!(get_json(&app, &format!("/messages/{id}")).await["message"]["notification"], false);
    eventually(async || !seen(&system, "a").await.failed.is_empty()).await;
    let failed = seen(&system, "a").await.failed;
    assert_eq!((failed.len(), failed[0].message_id.as_str()), (1, id.as_str()), "the watchers are told");
}
