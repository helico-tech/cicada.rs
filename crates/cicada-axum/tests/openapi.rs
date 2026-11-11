//! The two routers and the OpenAPI document: what is served, and that the committed copy is current.

mod common;

use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use cicada_axum::{actor_api, actor_api_read, openapi_document};
use common::*;
use serde_json::Value;

const WRITES: [&str; 4] = ["put", "post", "delete", "patch"];

fn committed() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/openapi.json"))
}

fn operations(document: &Value) -> Vec<(String, String)> {
    let paths = document["paths"].as_object().unwrap();
    paths.iter().flat_map(|(p, item)| item.as_object().unwrap().keys().map(|m| (m.clone(), p.clone()))).collect()
}

#[test]
fn the_committed_document_is_current() {
    let current = serde_json::to_string_pretty(&openapi_document()).unwrap() + "\n";
    if std::env::var("CICADA_UPDATE_OPENAPI").is_ok_and(|v| v == "true") {
        std::fs::write(committed(), &current).unwrap();
    }
    let on_disk = std::fs::read_to_string(committed()).unwrap_or_default();
    assert!(on_disk == current, "docs/openapi.json is out of date; run the tests with CICADA_UPDATE_OPENAPI=true");
}

#[test]
fn every_operation_has_an_id_and_answers_errors_as_problems() {
    let document = openapi_document();
    assert_eq!(document["openapi"], "3.1.0");
    for (method, path) in operations(&document) {
        let operation = &document["paths"][&path][&method];
        assert!(operation["operationId"].is_string(), "{method} {path}");
        assert!(operation["responses"]["default"]["content"][cicada_axum::PROBLEM_JSON].is_object(), "{method} {path}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_full_router_serves_the_whole_document() {
    let system = node_with(memory_store(), |b| b.host(CounterActor)).await;
    let app = actor_api(system);
    assert_eq!(get_json(&app, "/openapi.json").await, openapi_document());
}

#[tokio::test(start_paused = true)]
async fn the_read_router_has_no_route_that_changes_anything() {
    let system = node_with(memory_store(), |b| b.host(CounterActor)).await;
    let id = unique();
    system.create(&Counter { id: id.clone() }, &CounterState::default()).await.unwrap();
    let app = actor_api_read(system.clone());

    let document = get_json(&app, "/openapi.json").await;
    let served = operations(&document);
    assert!(!served.is_empty());
    assert!(served.iter().all(|(method, _)| !WRITES.contains(&method.as_str())), "{served:?}");

    let actor = format!("/actors/counter/{id}");
    assert_eq!(get(&app, &actor).await.status, StatusCode::OK);
    assert_eq!(get(&app, &format!("{actor}/children")).await.status, StatusCode::OK, "a read route");
    assert!(served.contains(&("get".to_owned(), "/actors/{type}/{key}/children".to_owned())), "{served:?}");
    assert_eq!(get(&app, "/watches").await.status, StatusCode::OK, "a read route");
    assert!(served.contains(&("get".to_owned(), "/watches".to_owned())), "{served:?}");
    let refused = [
        call(&app, Method::PUT, &actor, "{}").await,
        call(&app, Method::DELETE, &actor, "").await,
        call(&app, Method::DELETE, "/messages/x", "").await,
        call(&app, Method::DELETE, "/types/counter", "").await,
        call(&app, Method::PUT, "/watches", r#"{"target": "*", "watcher": "counter/x"}"#).await,
        call(&app, Method::DELETE, "/watches?target=*&watcher=counter%2Fx", "").await,
    ];
    for reply in refused {
        assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
    }
    // The write routes on their own paths simply do not exist here.
    let absent = [
        call(&app, Method::POST, &format!("{actor}/messages"), r#"{"type": "Add", "payload": {"n": 1}}"#).await,
        call(&app, Method::POST, &format!("{actor}/mailbox/pause"), "").await,
        call(&app, Method::POST, &format!("{actor}/mailbox/resume"), "").await,
        call(&app, Method::POST, &format!("{actor}/mailbox/purge"), "").await,
        call(&app, Method::POST, &format!("{actor}/turn/cancel"), "").await,
        call(&app, Method::POST, "/messages/x/cancel", "").await,
        call(&app, Method::POST, "/messages/x/retry", "").await,
        call(&app, Method::POST, "/messages/x/deliver-now", "").await,
    ];
    for reply in absent {
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.text);
    }
    // Not routes of the read router: `/messages/{id}` serves GET and nothing else.
    for path in ["/messages/bulk-cancel", "/messages/bulk-retry"] {
        assert_eq!(call(&app, Method::POST, path, "{}").await.status, StatusCode::METHOD_NOT_ALLOWED);
    }
    assert!(system.get_actor(&format!("counter/{id}").parse().unwrap()).await.unwrap().is_some());
}

#[tokio::test(start_paused = true)]
async fn one_type_is_served_by_name() {
    let system = node_with(memory_store(), |b| b.host(CounterActor)).await;
    let app = actor_api(system);
    let found = get_json(&app, "/types/counter").await;
    assert_eq!(found["name"], "counter");
    // Scenario: The view of a type.
    let keys: Vec<&str> = found.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["name", "hash", "state", "messages"]);
    assert_problem(&get(&app, "/types/nope").await, 404, cicada::api::problem_types::NO_SUCH_TYPE);
}
