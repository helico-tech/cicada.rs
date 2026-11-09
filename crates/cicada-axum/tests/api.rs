//! Every endpoint and status code of the HTTP contract on one hosting node, on virtual time.

mod common;

use std::collections::BTreeSet;

use axum::Router;
use axum::http::{StatusCode, header};
use cicada::ActorAddress;
use cicada::api::problem_types as pt;
use cicada::store::Store;
use cicada::time::{Timestamp, parse_instant};
use cicada_axum::actor_api;
use common::*;
use serde_json::{Value, json};

async fn app() -> Router {
    actor_api(node(memory_store()).await)
}

fn instant(v: &Value) -> Timestamp {
    parse_instant(v.as_str().expect("an instant")).unwrap()
}

// ---- PUT /actors/{path} ----

#[tokio::test(start_paused = true)]
async fn create_answers_201_then_200_and_leaves_the_existing_state_untouched() {
    let app = app().await;
    let first = put(&app, "/actors/counter/a", r#"{"state": {"total": 5}}"#).await;
    assert_eq!(first.status, StatusCode::CREATED);
    assert_eq!(first.content_type(), "application/json");
    assert_eq!(first.text, r#"{"uri":"actor://counter/a","created":true}"#);

    let again = put(&app, "/actors/counter/a", r#"{"state": {"total": 99}}"#).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.text, r#"{"uri":"actor://counter/a","created":false}"#);
    assert_eq!(get_json(&app, "/actors/counter/a").await["state"]["total"], 5);
}

#[tokio::test(start_paused = true)]
async fn create_without_body_and_the_state_types_defaults() {
    let app = app().await;
    assert_eq!(put(&app, "/actors/ping/p", "").await.status, StatusCode::CREATED);
    assert_eq!(put(&app, "/actors/counter/empty", r#"{"state": {}}"#).await.status, StatusCode::CREATED);
    // The state type is not optional, so a hosting node refuses a missing state.
    assert_eq!(put(&app, "/actors/counter/nostate", "").await.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(put(&app, "/actors/counter/nostate", "{}").await.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test(start_paused = true)]
async fn create_errors() {
    let app = app().await;
    assert_problem(&put(&app, "/actors/nope/a", "{}").await, 404, pt::NO_SUCH_TYPE);
    assert_problem(&put(&app, "/actors/counter/a", r#"{"state": {"total": "many"}}"#).await, 422, pt::INVALID_STATE);
    assert_problem(&put(&app, "/actors/ping/p", r#"{"state": {"x": 1}}"#).await, 422, pt::INVALID_STATE);
    assert_problem(&put(&app, "/actors/counter/a", r#"{"state": "#).await, 400, pt::BAD_REQUEST);
    assert_problem(&put(&app, "/actors/counter/a", "[1]").await, 400, pt::BAD_REQUEST);
}

// ---- GET / DELETE /actors/{path} ----

#[tokio::test(start_paused = true)]
async fn get_returns_type_params_state_and_version() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {"total": 1}}"#).await;
    let body = get_json(&app, "/actors/counter/a").await;
    assert_eq!(body["uri"], "actor://counter/a");
    assert_eq!(body["type"], "actor://counter/{id}");
    assert_eq!(body["params"]["id"], "a");
    assert_eq!(body["state"]["total"], 1);
    assert_eq!(body["version"], 0);
    instant(&body["createdAt"]);
    let keys: Vec<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["uri", "type", "params", "state", "version", "createdAt"]);

    put(&app, "/actors/ping/p", "").await;
    assert_eq!(get_json(&app, "/actors/ping/p").await["state"], Value::Null);

    assert_problem(&get(&app, "/actors/counter/missing").await, 404, pt::NO_SUCH_INSTANCE);
}

#[tokio::test(start_paused = true)]
async fn delete_answers_204_purges_the_mailbox_then_404() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let id = unique();
    post(
        &app,
        "/actors/counter/a/messages",
        &format!(r#"{{"id": "{id}", "type": "Add", "payload": {{"n": 1}}, "delay": "PT1H"}}"#),
    )
    .await;

    assert_eq!(delete(&app, "/actors/counter/a").await.status, StatusCode::NO_CONTENT);
    assert_problem(&get(&app, "/actors/counter/a").await, 404, pt::NO_SUCH_INSTANCE);
    assert_eq!(get_json(&app, &format!("/messages/{id}")).await["status"], "purged");
    assert_problem(&delete(&app, "/actors/counter/a").await, 404, pt::NO_SUCH_INSTANCE);
}

// ---- GET /actors?prefix= ----

#[tokio::test(start_paused = true)]
async fn list_is_paged_and_filtered_by_prefix() {
    let app = app().await;
    for n in ["a", "b", "c"] {
        put(&app, &format!("/actors/counter/{n}"), r#"{"state": {}}"#).await;
    }
    put(&app, "/actors/ping/p", "").await;

    let first = get_json(&app, "/actors?prefix=actor://counter/&limit=2").await;
    assert_eq!(items(&first), &vec![json!("actor://counter/a"), json!("actor://counter/b")]);
    let cursor = enc(first["next"].as_str().unwrap());
    let second = get_json(&app, &format!("/actors?prefix=actor://counter/&limit=2&cursor={cursor}")).await;
    assert_eq!(items(&second), &vec![json!("actor://counter/c")]);
    assert_eq!(second["next"], Value::Null);

    assert_eq!(items(&get_json(&app, "/actors?prefix=counter/").await).len(), 3);
    assert_eq!(items(&get_json(&app, "/actors").await).len(), 4);
    assert_problem(&get(&app, "/actors?limit=0").await, 400, pt::BAD_REQUEST);
    assert_problem(&get(&app, "/actors?limit=1001").await, 400, pt::BAD_REQUEST);
    assert_problem(&get(&app, "/actors?limit=x").await, 400, pt::BAD_REQUEST);
}

// ---- POST /actors/{path}/messages ----

#[tokio::test(start_paused = true)]
async fn send_answers_202_with_id_and_deliver_at_and_the_dispatch_loop_handles_it() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let response = post(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 4}}"#).await;
    assert_eq!(response.status, StatusCode::ACCEPTED);
    let body = response.json();
    let id = body["id"].as_str().unwrap().to_owned();
    assert_eq!((id.len(), id.matches('-').count()), (36, 4), "a UUID: {id}");
    instant(&body["deliverAt"]);

    eventually(async || get_json(&app, &format!("/messages/{id}")).await["status"] == "done").await;
    assert_eq!(get_json(&app, "/actors/counter/a").await["state"]["total"], 4);
}

#[tokio::test(start_paused = true)]
async fn send_is_idempotent_on_id() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let body = format!(r#"{{"id": "{}", "type": "Add", "payload": {{"n": 1}}, "delay": "PT1H"}}"#, unique());
    let first = post(&app, "/actors/counter/a/messages", &body).await;
    assert_eq!(first.status, StatusCode::ACCEPTED);
    let again = post(&app, "/actors/counter/a/messages", &body).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(first.json(), again.json());
    assert_eq!(items(&get_json(&app, "/actors/counter/a/messages").await).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn send_with_delay_deliver_at_correlation_id_and_headers() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let path = "/actors/counter/a/messages";
    let before = instant(
        &post(&app, path, r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT1H"}"#).await.json()["deliverAt"],
    );
    let millis = instant(
        &post(&app, path, r#"{"type": "Add", "payload": {"n": 1}, "delay": 7200000}"#).await.json()["deliverAt"],
    );
    let gap = (millis - before).num_minutes();
    assert!((59..=61).contains(&gap), "{gap}");
    let at = "2999-01-01T00:00:00Z";
    let at_response = post(
        &app,
        path,
        &format!(r#"{{"type": "Add", "payload": {{"n": 1}}, "deliverAt": "{at}", "correlationId": "c-1", "headers": {{"k": "v"}}}}"#),
    )
    .await
    .json();
    assert_eq!(instant(&at_response["deliverAt"]), parse_instant(at).unwrap());

    let page = get_json(&app, path).await;
    let last = items(&page).last().unwrap();
    assert_eq!(last["correlationId"], "c-1");
    assert_eq!(last["headers"]["k"], "v");
    assert_eq!(last["from"], Value::Null);
    assert!((instant(&last["deliverAt"]) - before).num_hours() > 100);
}

#[tokio::test(start_paused = true)]
async fn send_errors() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let path = "/actors/counter/a/messages";
    let add = r#"{"type": "Add", "payload": {"n": 1}}"#;
    assert_problem(&post(&app, "/actors/counter/missing/messages", add).await, 404, pt::NO_SUCH_INSTANCE);
    assert_problem(&post(&app, "/actors/nope/x/messages", add).await, 404, pt::NO_SUCH_INSTANCE);
    assert_problem(&post(&app, path, r#"{"type": "Nope", "payload": {}}"#).await, 422, pt::UNKNOWN_MESSAGE_TYPE);
    assert_problem(&post(&app, path, r#"{"type": "Add", "payload": {"n": "x"}}"#).await, 422, pt::INVALID_PAYLOAD);
    for bad in [
        r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT1S", "deliverAt": "2030-01-01T00:00:00Z"}"#,
        r#"{"type": "Add", "payload": {"n": 1}, "delay": "soon"}"#,
        r#"{"type": "Add", "payload": {"n": 1}, "deliverAt": "tomorrow"}"#,
        r#"{"type": "Add", "payload": {"n": 1}, "id": ""}"#,
        r#"{"payload": {"n": 1}}"#,
        r#"{"type": "Add", "#,
        "",
    ] {
        assert_problem(&post(&app, path, bad).await, 400, pt::BAD_REQUEST);
    }
    let not_allowed = post(&app, "/actors/counter/a", "{}").await;
    assert_problem(&not_allowed, 405, pt::METHOD_NOT_ALLOWED);
    assert_eq!(not_allowed.headers[header::ALLOW], "GET, PUT, DELETE");
}

// ---- GET /actors/{path}/messages ----

#[tokio::test(start_paused = true)]
async fn pending_lists_the_mailbox_in_delivery_order_paged() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    for (n, delay) in [(1, "PT3H"), (2, "PT1H"), (3, "PT2H")] {
        post(
            &app,
            "/actors/counter/a/messages",
            &format!(r#"{{"type": "Add", "payload": {{"n": {n}}}, "delay": "{delay}"}}"#),
        )
        .await;
    }
    let page = get_json(&app, "/actors/counter/a/messages?limit=2").await;
    let ns: Vec<&Value> = items(&page).iter().map(|m| &m["payload"]["n"]).collect();
    assert_eq!(ns, [&json!(2), &json!(3)]);
    let item = &items(&page)[0];
    assert_eq!(item["to"], "actor://counter/a");
    assert_eq!(item["type"], "Add");
    assert_eq!(item["attempt"], 0);
    let rest =
        get_json(&app, &format!("/actors/counter/a/messages?cursor={}", enc(page["next"].as_str().unwrap()))).await;
    assert_eq!(items(&rest).len(), 1);

    assert_problem(&get(&app, "/actors/counter/missing/messages").await, 404, pt::NO_SUCH_INSTANCE);
}

// ---- GET /messages ----

#[tokio::test(start_paused = true)]
async fn pending_of_all_instances_by_uri_each_mailbox_in_delivery_order_paged_and_by_prefix() {
    let app = app().await;
    for n in ["b", "a"] {
        put(&app, &format!("/actors/counter/{n}"), r#"{"state": {}}"#).await;
    }
    put(&app, "/actors/ping/p", "").await;
    for (path, n, delay) in [("b", 1, "PT1H"), ("a", 2, "PT2H"), ("a", 3, "PT1H")] {
        post(
            &app,
            &format!("/actors/counter/{path}/messages"),
            &format!(r#"{{"type": "Add", "payload": {{"n": {n}}}, "delay": "{delay}"}}"#),
        )
        .await;
    }
    post(&app, "/actors/ping/p/messages", r#"{"type": "Hello", "delay": "PT1H"}"#).await;

    let ns = |page: &Value| items(page).iter().map(|m| m["payload"]["n"].clone()).collect::<Vec<_>>();
    let first = get_json(&app, "/messages?prefix=counter/&limit=2").await;
    assert_eq!(ns(&first), [json!(3), json!(2)]); // actor://counter/a, earliest first
    let cursor = enc(first["next"].as_str().unwrap());
    let second = get_json(&app, &format!("/messages?prefix=counter/&limit=2&cursor={cursor}")).await;
    assert_eq!(ns(&second), [json!(1)]);
    assert_eq!(second["next"], Value::Null);

    let all = get_json(&app, "/messages").await;
    let to: Vec<&Value> = items(&all).iter().map(|m| &m["to"]).collect();
    assert_eq!(
        to,
        [
            &json!("actor://counter/a"),
            &json!("actor://counter/a"),
            &json!("actor://counter/b"),
            &json!("actor://ping/p")
        ]
    );
    assert_eq!(items(&get_json(&app, "/messages?prefix=nothing/").await).len(), 0);
    assert_problem(&get(&app, "/messages?cursor=nonsense").await, 400, pt::BAD_REQUEST);
    assert_problem(&get(&app, "/messages?limit=0").await, 400, pt::BAD_REQUEST);
}

// ---- GET / DELETE /messages/{id} ----

#[tokio::test(start_paused = true)]
async fn message_status_goes_pending_running_done_and_cancel_answers_409_for_running_and_finished() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let gate = unique();
    let blocked = send_id(
        &app,
        "/actors/counter/a/messages",
        &format!(r#"{{"type": "Block", "payload": {{"gate": "{gate}"}}}}"#),
    )
    .await;
    let queued = send_id(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 1}}"#).await;

    eventually(async || get_json(&app, &format!("/messages/{blocked}")).await["status"] == "running").await;
    let running = get_json(&app, &format!("/messages/{blocked}")).await;
    assert!(running["node"].is_string());
    instant(&running["leaseUntil"]);
    assert_eq!(running["message"]["attempt"], 1);
    assert_eq!(get_json(&app, &format!("/messages/{queued}")).await["status"], "pending");
    assert_problem(&delete(&app, &format!("/messages/{blocked}")).await, 409, pt::MESSAGE_RUNNING);

    gates::open(&gate);
    eventually(async || get_json(&app, &format!("/messages/{queued}")).await["status"] == "done").await;
    let done = get_json(&app, &format!("/messages/{blocked}")).await;
    assert_eq!(done["status"], "done");
    instant(&done["finishedAt"]);
    assert_problem(&delete(&app, &format!("/messages/{blocked}")).await, 409, pt::MESSAGE_FINISHED);
}

#[tokio::test(start_paused = true)]
async fn chain_shows_what_led_to_a_message_and_what_it_caused() {
    let app = app().await;
    for c in ["a", "b", "c"] {
        put(&app, &format!("/actors/counter/{c}"), r#"{"state": {}}"#).await;
    }
    let root =
        send_id(&app, "/actors/counter/a/messages", r#"{"type": "Forward", "payload": {"to": ["b", "c"], "n": 2}}"#)
            .await;
    eventually(async || get_json(&app, "/actors/counter/c").await["version"] == 1).await;
    eventually(async || get_json(&app, "/actors/counter/b").await["version"] == 1).await;

    let chain = get_json(&app, &format!("/messages/{root}/chain")).await;
    assert_eq!(chain["focus"], root.as_str());
    assert_eq!(chain["truncated"], false);
    let nodes = chain["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    let top = nodes.iter().find(|n| n["id"] == root.as_str()).unwrap();
    assert_eq!(top["causedBy"], Value::Null);
    assert_eq!(top["type"], "Forward");
    assert_eq!(top["status"], "done");
    let children: Vec<&Value> = nodes.iter().filter(|n| n["id"] != root.as_str()).collect();
    let targets: BTreeSet<&str> = children.iter().map(|c| c["to"].as_str().unwrap()).collect();
    assert_eq!(targets, BTreeSet::from(["actor://counter/b", "actor://counter/c"]));
    assert!(children.iter().all(|c| c["causedBy"] == root.as_str() && c["from"] == "actor://counter/a"));

    // From a child: its cause above it; and the envelope names the cause.
    let child = children[0]["id"].as_str().unwrap();
    let ids: BTreeSet<String> = get_json(&app, &format!("/messages/{child}/chain")).await["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, BTreeSet::from([root.clone(), child.to_owned()]));
    assert_eq!(get_json(&app, &format!("/messages/{child}")).await["message"]["causedBy"], root.as_str());

    assert_problem(&get(&app, &format!("/messages/{}/chain", unique())).await, 404, pt::NO_SUCH_MESSAGE);
}

#[tokio::test(start_paused = true)]
async fn cancel_a_pending_message_answers_204_and_unknown_ids_404() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let id =
        send_id(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT1H"}"#).await;
    assert_eq!(delete(&app, &format!("/messages/{id}")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(get_json(&app, &format!("/messages/{id}")).await["status"], "cancelled");
    assert_problem(&delete(&app, &format!("/messages/{id}")).await, 409, pt::MESSAGE_FINISHED);

    assert_problem(&get(&app, &format!("/messages/{}", unique())).await, 404, pt::NO_SUCH_MESSAGE);
    assert_problem(&delete(&app, &format!("/messages/{}", unique())).await, 404, pt::NO_SUCH_MESSAGE);
}

// ---- GET /failures ----

#[tokio::test(start_paused = true)]
async fn failures_list_failed_messages_with_envelope_and_error() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    put(&app, "/actors/counter/b", r#"{"state": {}}"#).await;
    let failed =
        send_id(&app, "/actors/counter/a/messages", r#"{"type": "Fail", "payload": {"reason": "kaput"}}"#).await;
    post(&app, "/actors/counter/b/messages", r#"{"type": "Fail", "payload": {"reason": "other"}}"#).await;
    eventually(async || items(&get_json(&app, "/failures").await).len() == 2).await;

    let status = get_json(&app, &format!("/messages/{failed}")).await;
    assert_eq!(status["status"], "failed");
    assert!(status["error"].as_str().unwrap().contains("kaput"));

    let page = get_json(&app, "/failures?prefix=actor://counter/a").await;
    let [only] = items(&page).as_slice() else { panic!("one failure: {page}") };
    assert_eq!(only["id"], failed.as_str());
    assert_eq!(only["actor"], "actor://counter/a");
    assert_eq!(only["status"], "failed");
    assert!(only["error"].as_str().unwrap().contains("kaput"));
    assert_eq!(only["message"]["type"], "Fail");

    assert_eq!(items(&get_json(&app, "/failures?limit=1").await).len(), 1);
    assert_eq!(items(&get_json(&app, "/failures?since=2999-01-01T00:00:00Z").await).len(), 0);
    assert_problem(&get(&app, "/failures?since=yesterday").await, 400, pt::BAD_REQUEST);
}

// ---- GET /types ----

#[tokio::test(start_paused = true)]
async fn types_return_the_generated_schemas() {
    let app = app().await;
    let list = get_json(&app, "/types").await;
    let types = items(&list);
    let templates: Vec<&Value> = types.iter().map(|t| &t["template"]).collect();
    assert_eq!(templates, [&json!("actor://counter/{id}"), &json!("actor://ping/{name}")]);
    let counter = &types[0];
    assert!(counter["hash"].as_str().unwrap().starts_with("sha256:"));
    let names: Vec<&str> = counter["messages"].as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(names, ["Add", "Block", "Fail", "Forward"]);
    assert_eq!(counter["messages"]["Add"]["properties"]["n"]["type"], "integer");
    assert_eq!(counter["state"]["type"], "object");
    assert_eq!(types[1]["state"], Value::Null);

    // The query value is URL-encoded once more: the actor URI itself contains %20.
    let found = get_json(&app, &format!("/types?uri={}", enc("actor://counter/x%20y"))).await;
    assert_eq!(found["type"]["template"], "actor://counter/{id}");
    assert_eq!(found["params"]["id"], "x y");

    assert_problem(&get(&app, "/types?uri=actor://nope/x").await, 404, pt::NO_SUCH_TYPE);
    assert_problem(&get(&app, "/types?uri=http://x").await, 400, pt::BAD_REQUEST);
}

// ---- GET /health and store failures ----

#[tokio::test(start_paused = true)]
async fn health_is_200_and_503_while_the_store_is_unavailable_as_are_sends() {
    let store = FlakyStore::new();
    let system = node(store.clone()).await;
    let node_id = system.node_id().to_string();
    let app = actor_api(system);
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let health = get_json(&app, "/health").await;
    assert_eq!(health, json!({"status": "up", "node": node_id, "state": "STARTED"}));

    store.set_down(true);
    assert_problem(&get(&app, "/health").await, 503, pt::UNHEALTHY);
    let send = post(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 1}}"#).await;
    assert_problem(&send, 503, pt::STORE_UNAVAILABLE);
    store.set_down(false);
    assert_eq!(get(&app, "/health").await.status, StatusCode::OK);
}

// ---- paths ----

#[tokio::test(start_paused = true)]
async fn percent_encoded_segments_map_to_canonical_actor_uris() {
    let app = app().await;
    assert_eq!(put(&app, "/actors/counter/a%20b%2Fc%C3%A9", r#"{"state": {}}"#).await.status, StatusCode::CREATED);
    let body = get_json(&app, "/actors/counter/a%20b%2Fc%C3%A9").await;
    assert_eq!(body["uri"], "actor://counter/a%20b%2Fc%C3%A9");
    assert_eq!(body["params"]["id"], "a b/cé");
    // Non-canonical but equivalent escapes address the same instance.
    assert_eq!(get(&app, "/actors/counter/a%20b%2fc%c3%a9").await.status, StatusCode::OK);
    assert_eq!(get(&app, "/actors/counter/a%20b%2Fc%C3%A9/messages").await.status, StatusCode::OK);
    assert_eq!(get_json(&app, "/actors").await["items"], json!(["actor://counter/a%20b%2Fc%C3%A9"]));
    assert_problem(&get(&app, "/actors/counter/a%zz").await, 400, pt::BAD_REQUEST);
}

#[tokio::test(start_paused = true)]
async fn a_trailing_messages_segment_is_the_mailbox_unless_escaped() {
    let app = app().await;
    assert_eq!(put(&app, "/actors/counter/messages", r#"{"state": {}}"#).await.status, StatusCode::CREATED);
    assert_eq!(get_json(&app, "/actors/counter/%6Dessages").await["uri"], "actor://counter/messages");
    // Unescaped, GET reads the mailbox of actor://counter, which does not exist.
    assert_problem(&get(&app, "/actors/counter/messages").await, 404, pt::NO_SUCH_INSTANCE);
    let sent =
        post(&app, "/actors/counter/%6Dessages/messages", r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT1H"}"#)
            .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED);
    assert_eq!(items(&get_json(&app, "/actors/counter/%6Dessages/messages").await).len(), 1);
    assert_eq!(ActorAddress::parse("actor://counter/messages").unwrap().to_http_path(), "/actors/counter/%6Dessages");
    assert_eq!(delete(&app, "/actors/counter/messages").await.status, StatusCode::NO_CONTENT);
}

#[tokio::test(start_paused = true)]
async fn dot_segments_are_addressed_as_themselves() {
    let app = app().await;
    for tenant in [".", ".."] {
        let address = ActorAddress::of([tenant]).unwrap();
        let counter =
            ActorAddress::parse(&format!("actor://counter/{}", &address.as_str()["actor://".len()..])).unwrap();
        let path = counter.to_http_path();
        assert_eq!(put(&app, &path, r#"{"state": {}}"#).await.status, StatusCode::CREATED, "{path}");
        assert_eq!(get_json(&app, &path).await["params"]["id"], tenant);
        assert_eq!(delete(&app, &path).await.status, StatusCode::NO_CONTENT);
    }
}

#[tokio::test(start_paused = true)]
async fn errors_carry_the_request_path_as_instance() {
    let app = app().await;
    let response = get(&app, "/actors/counter/zz?x=1").await;
    assert_eq!(response.json()["instance"], "/actors/counter/zz?x=1");
    assert!(response.headers.get(header::WWW_AUTHENTICATE).is_none());
}

#[tokio::test(start_paused = true)]
async fn the_store_outlives_the_router() {
    // The router holds the node; the store stays usable by the host after the router is dropped.
    let store = memory_store();
    let app = actor_api(node(store.clone()).await);
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    drop(app);
    assert_eq!(store.list("", None, 10).await.unwrap().items.len(), 1);
}
