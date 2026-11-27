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
fn stats_documents_an_optional_failed_count() {
    let document = openapi_document();
    let response = &document["paths"]["/stats"]["get"]["responses"]["200"];
    assert_eq!(response["content"]["application/json"]["schema"]["$ref"], "#/components/schemas/StatsView");
    let stats = &document["components"]["schemas"]["StatsView"];
    assert_eq!(stats["properties"]["failed"]["type"], serde_json::json!(["integer", "null"]));
    assert_eq!(stats["properties"]["failed"]["minimum"], 0);
    assert!(!stats["required"].as_array().unwrap().iter().any(|field| field == "failed"));
}

/// remote-turn-http, Scenarios: Documented limits match the checks; A limit changes without its documentation.
#[test]
fn the_lease_request_documents_the_limits_the_core_checks() {
    use cicada::runtime::{MAX_LEASE_BATCH, MAX_LEASE_TYPES, limits::MAX_ID_LENGTH};
    let document = openapi_document();
    let properties = &document["components"]["schemas"]["LeaseTurnsRequest"]["properties"];
    // Each phrase ends with the word or punctuation after its number, so an expected 100 cannot match 1000.
    for (property, phrase) in [
        ("owner", format!(" at most {MAX_ID_LENGTH} characters,")),
        ("types", format!(" 1 to {MAX_LEASE_TYPES} types (`MAX_LEASE_TYPES`).")),
        ("limit", format!(" 1 to {MAX_LEASE_BATCH} (`MAX_LEASE_BATCH`).")),
    ] {
        let description = properties[property]["description"].as_str().unwrap_or_default();
        assert!(
            description.contains(&phrase),
            "LeaseTurnsRequest.{property} should contain {phrase:?}, its description is {description:?}"
        );
    }
}

/// conformance-fixtures, Scenario: Status classification (the success rows of `fixtures/status.json`, and the follow-up
/// of a registration, are the documented success responses of their routes).
#[test]
fn the_success_statuses_of_the_fixtures_are_the_documented_ones() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/status.json");
    let status: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(status["format"], 1);
    let document = openapi_document();
    let follow_ups = status["registration"].as_array().unwrap().iter().map(|r| &r["followUp"]);
    let success = status["success"].as_object().unwrap().values();
    let mut checked = 0;
    for row in success.chain(follow_ups.filter(|f| f["status"] == 200)) {
        let (method, route) = (row["method"].as_str().unwrap().to_lowercase(), row["path"].as_str().unwrap());
        let operation = &document["paths"][route][&method];
        assert_eq!(operation["operationId"], row["operationId"], "{method} {route}");
        let documented: Vec<&String> = operation["responses"].as_object().unwrap().keys().collect();
        let code = row["status"].to_string();
        assert_eq!(documented, ["default", code.as_str()], "{method} {route} succeeds with {code} only");
        checked += 1;
    }
    assert_eq!(checked, 7, "six operations and the follow-up of a conflict");
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

/// remote-turn-http, Scenario: Published contract and library example (the four operations, their schemas and the
/// required fields of the execution message).
#[tokio::test(start_paused = true)]
async fn only_the_full_document_describes_the_remote_turn_operations() {
    let system = node_with(memory_store(), |b| b).await;
    let full = get_json(&actor_api(system.clone()), "/openapi.json").await;
    assert_eq!(full, openapi_document());
    let schemas = &full["components"]["schemas"];
    for (path, id, request, response) in [
        ("/leases", "leaseTurns", "LeaseTurnsRequest", Some("LeaseTurnsResponse")),
        ("/leases/renew", "renewTurn", "RenewTurnRequest", Some("RenewTurnResponse")),
        ("/leases/release", "releaseTurn", "ReleaseTurnRequest", None),
        ("/leases/complete", "completeTurn", "CompleteTurnRequest", Some("CompleteTurnResponse")),
    ] {
        let item = full["paths"][path].as_object().unwrap();
        assert_eq!(item.keys().collect::<Vec<_>>(), ["post"], "{path}");
        let op = &item["post"];
        assert_eq!(op["operationId"], id);
        let body = &op["requestBody"]["content"]["application/json"]["schema"]["$ref"];
        assert_eq!(body, &format!("#/components/schemas/{request}"));
        assert_eq!(schemas[request]["additionalProperties"], false, "{request} is strict");
        match response {
            Some(r) => {
                let ok = &op["responses"]["200"]["content"]["application/json"]["schema"]["$ref"];
                assert_eq!(ok, &format!("#/components/schemas/{r}"));
            }
            None => assert!(op["responses"]["204"].is_object()),
        }
        assert!(op["responses"]["default"]["content"][cicada_axum::PROBLEM_JSON].is_object());
    }
    let required = |name: &str| {
        let mut r: Vec<String> =
            schemas[name]["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
        r.sort();
        r
    };
    let mut message = [
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
        "notification",
    ];
    message.sort();
    assert_eq!(required("TurnMessageView"), message, "nullable fields are required too");
    assert_eq!(required("TurnSendView"), ["id", "payload", "to", "type"]);
    assert_eq!(required("CompleteTurnRequest"), ["actor", "epoch", "messageId", "report"]);
    assert!(schemas["TurnEffectsView"].get("required").is_none_or(|r| r.as_array().unwrap().is_empty()));

    let read = get_json(&actor_api_read(system), "/openapi.json").await;
    assert!(read["paths"].as_object().unwrap().keys().all(|p| !p.starts_with("/leases")));
}

/// Scenarios: The generated contract and client describe the new route; The address listing stays compatible.
#[tokio::test(start_paused = true)]
async fn both_routers_describe_the_state_listing_and_leave_the_address_listing_as_it_was() {
    let system = node_with(memory_store(), |b| b.host(CounterActor)).await;
    for app in [actor_api(system.clone()), actor_api_read(system.clone())] {
        let document = get_json(&app, "/openapi.json").await;
        let states = &document["paths"]["/actors/states"]["get"];
        assert_eq!(states["operationId"], "listActorStates");
        let parameters: Vec<&str> =
            states["parameters"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert_eq!(parameters, ["prefix", "cursor", "limit"]);
        let ok = &states["responses"]["200"]["content"]["application/json"]["schema"]["$ref"];
        assert_eq!(ok, "#/components/schemas/ActorViewPage");
        let page = &document["components"]["schemas"]["ActorViewPage"]["properties"];
        assert_eq!(page["items"]["items"]["$ref"], "#/components/schemas/ActorView");
        assert!(states["responses"]["default"]["content"][cicada_axum::PROBLEM_JSON].is_object());
        let actors = &document["paths"]["/actors"]["get"];
        assert_eq!(actors["operationId"], "listActors");
        let ok = &actors["responses"]["200"]["content"]["application/json"]["schema"]["$ref"];
        assert_eq!(ok, "#/components/schemas/ActorAddressPage", "no union with the state listing");
    }
}

/// Scenario: Generated documents.
#[tokio::test(start_paused = true)]
async fn both_routers_describe_the_change_stream_as_server_sent_events_with_only_a_prefix() {
    let system = node_with(memory_store(), |b| b.host(CounterActor)).await;
    for app in [actor_api(system.clone()), actor_api_read(system.clone())] {
        let document = get_json(&app, "/openapi.json").await;
        let changes = &document["paths"]["/changes"]["get"];
        assert_eq!(changes["operationId"], "streamActorChanges");
        let parameters: Vec<&str> =
            changes["parameters"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert_eq!(parameters, ["prefix"], "no limit or cursor");
        let ok = &changes["responses"]["200"];
        let media: Vec<&String> = ok["content"].as_object().unwrap().keys().collect();
        assert_eq!(media, ["text/event-stream"], "no JSON page");
        let stream = &ok["content"]["text/event-stream"];
        assert_eq!(stream["schema"]["type"], "string");
        let example = stream["example"].as_str().unwrap();
        assert!(example.contains("event: resync\ndata: {}\n\n"), "{example}");
        assert!(example.contains("event: changed\ndata: {\"address\":\"order/42\"}\n\n"), "{example}");
        let description = ok["description"].as_str().unwrap();
        for words in ["keep-alive", "Last-Event-ID", "no replay", "store is closed", "first event is `resync`"] {
            assert!(description.contains(words), "{words}: {description}");
        }
        assert!(changes["responses"]["default"]["content"][cicada_axum::PROBLEM_JSON].is_object());
    }
}

/// type-registry, Scenario: Generated API description.
#[tokio::test(start_paused = true)]
async fn only_the_full_document_describes_the_registration_of_a_type() {
    let system = node_with(memory_store(), |b| b).await;
    let full = get_json(&actor_api(system.clone()), "/openapi.json").await;
    let register = &full["paths"]["/types/{name}"]["put"];
    assert_eq!(register["operationId"], "registerType");
    let parameters: Vec<&str> =
        register["parameters"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(parameters, ["name"]);
    let body = &register["requestBody"]["content"]["application/json"]["schema"]["$ref"];
    assert_eq!(body, "#/components/schemas/RegisterTypeRequest");
    let request = &full["components"]["schemas"]["RegisterTypeRequest"];
    assert_eq!(request["required"], serde_json::json!(["state", "messages"]));
    assert_eq!(request["additionalProperties"], false);
    let ok = &register["responses"]["200"]["content"]["application/json"]["schema"]["$ref"];
    assert_eq!(ok, "#/components/schemas/TypeView");
    assert!(register["responses"]["default"]["content"][cicada_axum::PROBLEM_JSON].is_object());
    assert_eq!(full, openapi_document());

    let read = get_json(&actor_api_read(system), "/openapi.json").await;
    let methods: Vec<&String> = read["paths"]["/types/{name}"].as_object().unwrap().keys().collect();
    assert_eq!(methods, ["get"]);
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
    assert_eq!(items(&get_json(&app, "/actors/states").await)[0]["address"], format!("counter/{id}"), "a read route");
    assert!(served.contains(&("get".to_owned(), "/actors/states".to_owned())), "{served:?}");
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
