//! Every endpoint and status code of the HTTP contract on one hosting node, on virtual time.

mod common;

use axum::Router;
use axum::http::{StatusCode, header};
use cicada::api::problem_types as pt;
use cicada::prelude::*;
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
    assert_eq!(first.text, r#"{"address":"counter/a","created":true}"#);

    let again = put(&app, "/actors/counter/a", r#"{"state": {"total": 99}}"#).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.text, r#"{"address":"counter/a","created":false}"#);
    assert_eq!(get_json(&app, "/actors/counter/a").await["state"]["total"], 5);
}

// ---- a state is required and never null ----

const CLEANUP: &str = "/actors/maintenance.cleanup/_";

/// A node hosting `counter` and `maintenance.cleanup`, whose state is `NoState`.
async fn cleanup_node() -> ActorSystem {
    node_with(memory_store(), |b| b.host(CounterActor).host(CleanupActor)).await
}

/// Asserts that a refused create left no instance at `path`.
async fn absent(app: &Router, path: &str) {
    assert_problem(&get(app, path).await, 404, pt::NO_SUCH_INSTANCE);
}

#[tokio::test(start_paused = true)]
async fn a_create_over_http_with_a_state() {
    let app = actor_api(cleanup_node().await);
    assert_eq!(put(&app, CLEANUP, r#"{"state": {}}"#).await.status, StatusCode::CREATED);
    assert_eq!(get_json(&app, CLEANUP).await["state"], json!({}));
    assert_eq!(put(&app, "/actors/counter/empty", r#"{"state": {}}"#).await.status, StatusCode::CREATED);
}

#[tokio::test(start_paused = true)]
async fn a_create_over_http_without_a_body_is_a_bad_request() {
    let app = actor_api(cleanup_node().await);
    let reply = put(&app, CLEANUP, "").await;
    assert_problem(&reply, 400, pt::BAD_REQUEST);
    assert!(reply.text.contains("state"), "{}", reply.text);
    absent(&app, CLEANUP).await;
}

#[tokio::test(start_paused = true)]
async fn a_create_over_http_without_the_state_field_is_a_bad_request() {
    let app = actor_api(cleanup_node().await);
    let reply = put(&app, CLEANUP, "{}").await;
    assert_problem(&reply, 400, pt::BAD_REQUEST);
    assert!(reply.text.contains("missing field `state`"), "{}", reply.text);
    absent(&app, CLEANUP).await;
    assert_problem(&put(&app, "/actors/counter/a", "{}").await, 400, pt::BAD_REQUEST);
}

#[tokio::test(start_paused = true)]
async fn a_null_state_over_http_is_an_invalid_state() {
    let app = actor_api(cleanup_node().await);
    for path in ["/actors/counter/a", CLEANUP] {
        assert_problem(&put(&app, path, r#"{"state": null}"#).await, 422, pt::INVALID_STATE);
        absent(&app, path).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_state_with_a_field_for_a_type_that_keeps_no_data_is_an_invalid_state() {
    let app = actor_api(cleanup_node().await);
    assert_problem(&put(&app, CLEANUP, r#"{"state": {"oops": 1}}"#).await, 422, pt::INVALID_STATE);
    absent(&app, CLEANUP).await;
}

#[tokio::test(start_paused = true)]
async fn an_actor_of_a_type_that_keeps_no_data_has_an_empty_state() {
    let system = cleanup_node().await;
    system.create(&Cleanup, &NoState {}).await.unwrap();
    let app = actor_api(system);
    let body = get_json(&app, CLEANUP).await;
    assert_eq!(body["state"], json!({}));
    assert_eq!(body["version"], 0);
}

#[tokio::test(start_paused = true)]
async fn a_type_that_keeps_no_data_has_the_schema_of_an_object_without_properties() {
    let app = actor_api(cleanup_node().await);
    let state = get_json(&app, "/types/maintenance.cleanup").await["state"].clone();
    assert_eq!(state["type"], "object", "{state}");
    assert!(state.get("properties").is_none_or(|p| p == &json!({})), "{state}");
    assert_eq!(state["additionalProperties"], false, "{state}");
}

#[tokio::test(start_paused = true)]
async fn create_errors() {
    let app = app().await;
    assert_problem(&put(&app, "/actors/nope/a", r#"{"state": {}}"#).await, 404, pt::NO_SUCH_TYPE);
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
    assert_eq!(body["address"], "counter/a");
    assert_eq!(body["type"], "counter");
    assert_eq!(body["params"]["id"], "a");
    assert_eq!(body["state"]["total"], 1);
    assert_eq!(body["version"], 0);
    instant(&body["createdAt"]);
    let keys: Vec<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["address", "type", "params", "state", "version", "createdAt", "paused"]);

    put(&app, "/actors/ping/p", r#"{"state": {}}"#).await;
    assert_eq!(get_json(&app, "/actors/ping/p").await["state"], json!({}));

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
    put(&app, "/actors/ping/p", r#"{"state": {}}"#).await;

    let first = get_json(&app, "/actors?prefix=counter/&limit=2").await;
    assert_eq!(items(&first), &vec![json!("counter/a"), json!("counter/b")]);
    let cursor = enc(first["next"].as_str().unwrap());
    let second = get_json(&app, &format!("/actors?prefix=counter/&limit=2&cursor={cursor}")).await;
    assert_eq!(items(&second), &vec![json!("counter/c")]);
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
    assert_eq!(items(&get_json(&app, "/actors/counter/a/mailbox").await).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn send_with_delay_deliver_at_correlation_id_and_headers() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let path = "/actors/counter/a/messages";
    let before = instant(
        &post(&app, path, r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT1H"}"#).await.json()["deliverAt"],
    );
    let later = instant(
        &post(&app, path, r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT2H"}"#).await.json()["deliverAt"],
    );
    let gap = (later - before).num_minutes();
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

    let page = get_json(&app, "/actors/counter/a/mailbox").await;
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
        // A duration is an ISO-8601 string and not negative.
        r#"{"type": "Add", "payload": {"n": 1}, "delay": "5s"}"#,
        r#"{"type": "Add", "payload": {"n": 1}, "delay": 7200000}"#,
        r#"{"type": "Add", "payload": {"n": 1}, "delay": "-PT1S"}"#,
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
    let page = get_json(&app, "/actors/counter/a/mailbox?limit=2").await;
    let ns: Vec<&Value> = items(&page).iter().map(|m| &m["payload"]["n"]).collect();
    assert_eq!(ns, [&json!(2), &json!(3)]);
    let item = &items(&page)[0];
    assert_eq!(item["to"], "counter/a");
    assert_eq!(item["type"], "Add");
    assert_eq!(item["attempt"], 0);
    let rest =
        get_json(&app, &format!("/actors/counter/a/mailbox?cursor={}", enc(page["next"].as_str().unwrap()))).await;
    assert_eq!(items(&rest).len(), 1);

    assert_problem(&get(&app, "/actors/counter/missing/mailbox").await, 404, pt::NO_SUCH_INSTANCE);
}

// ---- GET /messages ----

#[tokio::test(start_paused = true)]
async fn pending_of_all_instances_by_uri_each_mailbox_in_delivery_order_paged_and_by_prefix() {
    let app = app().await;
    for n in ["b", "a"] {
        put(&app, &format!("/actors/counter/{n}"), r#"{"state": {}}"#).await;
    }
    put(&app, "/actors/ping/p", r#"{"state": {}}"#).await;
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
    assert_eq!(ns(&first), [json!(3), json!(2)]); // counter/a, earliest first
    let cursor = enc(first["next"].as_str().unwrap());
    let second = get_json(&app, &format!("/messages?prefix=counter/&limit=2&cursor={cursor}")).await;
    assert_eq!(ns(&second), [json!(1)]);
    assert_eq!(second["next"], Value::Null);

    let all = get_json(&app, "/messages").await;
    let to: Vec<&Value> = items(&all).iter().map(|m| &m["to"]).collect();
    assert_eq!(to, [&json!("counter/a"), &json!("counter/a"), &json!("counter/b"), &json!("ping/p")]);
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
    assert_problem(&post(&app, &format!("/messages/{blocked}/cancel"), "").await, 409, pt::MESSAGE_RUNNING);

    gates::open(&gate);
    eventually(async || get_json(&app, &format!("/messages/{queued}")).await["status"] == "done").await;
    let done = get_json(&app, &format!("/messages/{blocked}")).await;
    assert_eq!(done["status"], "done");
    instant(&done["finishedAt"]);
    assert_problem(&post(&app, &format!("/messages/{blocked}/cancel"), "").await, 409, pt::MESSAGE_FINISHED);
}

#[tokio::test(start_paused = true)]
async fn a_message_sent_by_a_turn_names_its_sender_and_its_cause() {
    let app = app().await;
    for c in ["a", "b"] {
        put(&app, &format!("/actors/counter/{c}"), r#"{"state": {}}"#).await;
    }
    // b is busy, so what a sends to it stays in its mailbox.
    let gate = unique();
    let blocked = send_id(
        &app,
        "/actors/counter/b/messages",
        &format!(r#"{{"type": "Block", "payload": {{"gate": "{gate}"}}}}"#),
    )
    .await;
    eventually(async || get_json(&app, &format!("/messages/{blocked}")).await["status"] == "running").await;
    let root =
        send_id(&app, "/actors/counter/a/messages", r#"{"type": "Forward", "payload": {"to": ["b"], "n": 2}}"#).await;
    eventually(async || get_json(&app, &format!("/messages/{root}")).await["status"] == "done").await;

    let top = get_json(&app, &format!("/messages/{root}")).await;
    assert_eq!(top["message"]["causedBy"], Value::Null);
    assert_eq!(top["message"]["from"], Value::Null);
    let mailbox = get_json(&app, "/actors/counter/b/mailbox").await;
    let sent: Vec<&Value> = items(&mailbox).iter().filter(|m| m["type"] == "Add").collect();
    let [child] = sent.as_slice() else { panic!("one message from a: {mailbox}") };
    assert_eq!(child["causedBy"], root.as_str());
    assert_eq!(child["from"], "counter/a");
    let child_id = child["id"].as_str().unwrap();
    assert_eq!(get_json(&app, &format!("/messages/{child_id}")).await["message"]["causedBy"], root.as_str());

    gates::open(&gate);
    eventually(async || get_json(&app, "/actors/counter/b").await["state"]["total"] == 2).await;
    assert_eq!(get_json(&app, &format!("/messages/{child_id}")).await["message"]["causedBy"], root.as_str());
}

#[tokio::test(start_paused = true)]
async fn cancel_a_pending_message_answers_204_and_unknown_ids_404() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let id =
        send_id(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT1H"}"#).await;
    assert_eq!(post(&app, &format!("/messages/{id}/cancel"), "").await.status, StatusCode::NO_CONTENT);
    assert_eq!(get_json(&app, &format!("/messages/{id}")).await["status"], "cancelled");
    assert_problem(&post(&app, &format!("/messages/{id}/cancel"), "").await, 409, pt::MESSAGE_FINISHED);

    assert_problem(&get(&app, &format!("/messages/{}", unique())).await, 404, pt::NO_SUCH_MESSAGE);
    assert_problem(&post(&app, &format!("/messages/{}/cancel", unique()), "").await, 404, pt::NO_SUCH_MESSAGE);
}

// ---- GET /messages/{id}/caused ----

#[tokio::test(start_paused = true)]
async fn caused_lists_the_messages_a_turn_sent_one_level_down_with_paging() {
    let app = app().await;
    for key in ["a", "b", "c"] {
        put(&app, &format!("/actors/counter/{key}"), r#"{"state": {}}"#).await;
    }
    let forward =
        send_id(&app, "/actors/counter/a/messages", r#"{"type": "Forward", "payload": {"to": ["b", "c"], "n": 1}}"#)
            .await;
    eventually(async || get_json(&app, "/actors/counter/c").await["state"]["total"] == 1).await;

    let page = get_json(&app, &format!("/messages/{forward}/caused")).await;
    let mut children: Vec<&str> = items(&page).iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(children.len(), 2, "{page}");
    assert!(children.is_sorted(), "in message-id order");
    for child in items(&page) {
        assert_eq!(child["message"]["causedBy"], forward.as_str());
        assert_eq!(child["status"], "done");
        assert!(["counter/b", "counter/c"].contains(&child["message"]["to"].as_str().unwrap()));
    }
    // Paging by cursor.
    let first = get_json(&app, &format!("/messages/{forward}/caused?limit=1")).await;
    assert_eq!(items(&first).len(), 1);
    let cursor = first["next"].as_str().expect("a next page").to_owned();
    let second = get_json(&app, &format!("/messages/{forward}/caused?limit=1&cursor={}", enc(&cursor))).await;
    assert_eq!(second["next"], serde_json::Value::Null);
    children = vec![items(&first)[0]["id"].as_str().unwrap(), items(&second)[0]["id"].as_str().unwrap()];
    assert!(children[0] < children[1]);
    // A message that caused nothing has an empty list; an unknown one is a 404.
    let leaf = children[0];
    assert!(items(&get_json(&app, &format!("/messages/{leaf}/caused")).await).is_empty());
    assert_problem(&get(&app, &format!("/messages/{}/caused", unique())).await, 404, pt::NO_SUCH_MESSAGE);
    assert_problem(&get(&app, &format!("/messages/{forward}/caused?limit=0")).await, 400, pt::BAD_REQUEST);
}

// ---- GET /outcomes ----

#[tokio::test(start_paused = true)]
async fn outcomes_list_finished_messages_with_envelope_and_error() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    put(&app, "/actors/counter/b", r#"{"state": {}}"#).await;
    let failed =
        send_id(&app, "/actors/counter/a/messages", r#"{"type": "Fail", "payload": {"reason": "kaput"}}"#).await;
    post(&app, "/actors/counter/b/messages", r#"{"type": "Fail", "payload": {"reason": "other"}}"#).await;
    eventually(async || items(&get_json(&app, "/outcomes?status=failed").await).len() == 2).await;

    let status = get_json(&app, &format!("/messages/{failed}")).await;
    assert_eq!(status["status"], "failed");
    assert!(status["error"].as_str().unwrap().contains("kaput"));

    let page = get_json(&app, "/outcomes?status=failed&prefix=counter/a").await;
    let [only] = items(&page).as_slice() else { panic!("one failure: {page}") };
    assert_eq!(only["id"], failed.as_str());
    assert_eq!(only["actor"], "counter/a");
    assert_eq!(only["status"], "failed");
    assert!(only["error"].as_str().unwrap().contains("kaput"));
    assert_eq!(only["message"]["type"], "Fail");

    assert_eq!(items(&get_json(&app, "/outcomes?status=failed&limit=1").await).len(), 1);
    assert_eq!(items(&get_json(&app, "/outcomes?status=failed&since=2999-01-01T00:00:00Z").await).len(), 0);
    assert_problem(&get(&app, "/outcomes?status=failed&since=yesterday").await, 400, pt::BAD_REQUEST);

    // Other statuses: a done message is not a failure, and shows without a status or with `status=done`.
    let done = send_id(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 1}}"#).await;
    eventually(async || items(&get_json(&app, "/outcomes?status=done").await).len() == 1).await;
    let page = get_json(&app, "/outcomes?status=done").await;
    let [only] = items(&page).as_slice() else { panic!("one done: {page}") };
    assert_eq!((only["id"].as_str(), only["status"].as_str()), (Some(done.as_str()), Some("done")));
    assert_eq!(items(&get_json(&app, "/outcomes").await).len(), 3);
    assert_eq!(items(&get_json(&app, "/outcomes?status=failed").await).len(), 2);
    assert_eq!(items(&get_json(&app, "/outcomes?status=cancelled").await).len(), 0);
    assert_problem(&get(&app, "/outcomes?status=pending").await, 400, pt::BAD_REQUEST);
    assert_problem(&get(&app, "/outcomes?status=").await, 400, pt::BAD_REQUEST);
}

// ---- GET /stats ----

#[tokio::test(start_paused = true)]
async fn stats_count_actors_and_pending_messages_and_name_the_oldest_due_one() {
    let app = app().await;
    let empty = get_json(&app, "/stats").await;
    assert_eq!(empty, serde_json::json!({"actors": 0, "pending": 0, "oldestDue": null}));
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    put(&app, "/actors/counter/b", r#"{"state": {}}"#).await;
    send_id(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT1H"}"#).await;
    send_id(&app, "/actors/counter/b/messages", r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT2H"}"#).await;

    let all = get_json(&app, "/stats").await;
    assert_eq!((all["actors"].as_u64(), all["pending"].as_u64()), (Some(2), Some(2)));
    assert_eq!(all["oldestDue"], serde_json::Value::Null, "nothing is due yet");
    let one = get_json(&app, "/stats?prefix=counter/a").await;
    assert_eq!((one["actors"].as_u64(), one["pending"].as_u64()), (Some(1), Some(1)));
    assert_eq!(get_json(&app, "/stats?prefix=other/").await["actors"], 0);
}

// ---- /types ----

#[tokio::test(start_paused = true)]
async fn delete_a_type_answers_204_404_and_409_while_actors_of_it_exist() {
    let app = app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let in_use = delete(&app, "/types/counter").await;
    assert_problem(&in_use, 409, pt::TYPE_IN_USE);
    assert_eq!(in_use.json()["instance"], "/types/counter");
    assert_eq!(get(&app, "/types/counter").await.status, StatusCode::OK, "an in-use type stays");

    assert_problem(&delete(&app, "/types/nope").await, 404, pt::NO_SUCH_TYPE);
    assert_problem(&delete(&app, "/types/Not%20A%20Name").await, 404, pt::NO_SUCH_TYPE);
    assert_eq!(delete(&app, "/actors/counter/a").await.status, StatusCode::NO_CONTENT);
    assert_eq!(delete(&app, "/types/counter").await.status, StatusCode::NO_CONTENT);
    assert_problem(&get(&app, "/types/counter").await, 404, pt::NO_SUCH_TYPE);
    assert!(items(&get_json(&app, "/types").await).iter().all(|t| t["name"] != "counter"));
    assert_problem(&delete(&app, "/types/counter").await, 404, pt::NO_SUCH_TYPE);
    // The type is gone for creates, too.
    assert_problem(&put(&app, "/actors/counter/b", r#"{"state": {}}"#).await, 404, pt::NO_SUCH_TYPE);
}

#[tokio::test(start_paused = true)]
async fn types_return_the_generated_schemas() {
    let app = app().await;
    let list = get_json(&app, "/types").await;
    let types = items(&list);
    let names: Vec<&Value> = types.iter().map(|t| &t["name"]).collect();
    assert_eq!(names, [&json!("counter"), &json!("ping")]);
    assert_eq!(types[0]["key"], json!(["id"]));
    let counter = &types[0];
    assert!(counter["hash"].as_str().unwrap().starts_with("sha256:"));
    let messages: Vec<&str> = counter["messages"].as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(messages, ["Add", "Block", "Fail", "Forward"]);
    assert_eq!(counter["messages"]["Add"]["properties"]["n"]["type"], "integer");
    assert_eq!(counter["state"]["type"], "object");
    assert_eq!(types[1]["state"]["type"], "object");

    // The list takes no query parameters; one that is sent is ignored.
    assert_eq!(get_json(&app, "/types?uri=nope/x").await, list);
}

// ---- GET /health and store failures ----

#[tokio::test(start_paused = true)]
async fn health_is_200_and_503_after_the_store_is_closed_as_are_sends() {
    let store = memory_store();
    let system = node(store.clone()).await;
    let node_id = system.node_id().to_string();
    let app = actor_api(system);
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let health = get_json(&app, "/health").await;
    assert_eq!(health, json!({"status": "up", "node": node_id, "state": "STARTED"}));

    store.close().await;
    assert_problem(&get(&app, "/health").await, 503, pt::UNHEALTHY);
    let send = post(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 1}}"#).await;
    assert_problem(&send, 503, pt::STORE_UNAVAILABLE);
}

// ---- paths ----

#[tokio::test(start_paused = true)]
async fn percent_encoded_segments_map_to_canonical_actor_addresses() {
    let app = app().await;
    assert_eq!(put(&app, "/actors/counter/a%20b%2Fc%C3%A9", r#"{"state": {}}"#).await.status, StatusCode::CREATED);
    let body = get_json(&app, "/actors/counter/a%20b%2Fc%C3%A9").await;
    assert_eq!(body["address"], "counter/a%20b%2Fc%C3%A9");
    assert_eq!(body["params"]["id"], "a b/cé");
    // Non-canonical but equivalent escapes address the same instance.
    assert_eq!(get(&app, "/actors/counter/a%20b%2fc%c3%a9").await.status, StatusCode::OK);
    assert_eq!(get(&app, "/actors/counter/a%20b%2Fc%C3%A9/mailbox").await.status, StatusCode::OK);
    assert_eq!(get_json(&app, "/actors").await["items"], json!(["counter/a%20b%2Fc%C3%A9"]));
    assert_problem(&get(&app, "/actors/counter/a%zz").await, 400, pt::BAD_REQUEST);
}

#[tokio::test(start_paused = true)]
async fn a_key_called_messages_is_the_actor_and_the_next_segment_is_its_mailbox() {
    let app = app().await;
    assert_eq!(put(&app, "/actors/counter/messages", r#"{"state": {}}"#).await.status, StatusCode::CREATED);
    assert_eq!(get_json(&app, "/actors/counter/messages").await["address"], "counter/messages");
    let sent =
        post(&app, "/actors/counter/messages/messages", r#"{"type": "Add", "payload": {"n": 1}, "delay": "PT1H"}"#)
            .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED);
    assert_eq!(items(&get_json(&app, "/actors/counter/messages/mailbox").await).len(), 1);
    assert_eq!(delete(&app, "/actors/counter/messages").await.status, StatusCode::NO_CONTENT);
}

#[tokio::test(start_paused = true)]
async fn the_keys_dot_and_dot_dot_are_refused_and_a_third_path_level_is_not_a_route() {
    let app = app().await;
    for key in ["%2E", "%2E%2E"] {
        let path = format!("/actors/counter/{key}");
        assert_problem(&put(&app, &path, r#"{"state": {}}"#).await, 400, pt::BAD_REQUEST);
    }
    assert_eq!(get(&app, "/actors/a/b/c").await.status, StatusCode::NOT_FOUND);
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
    let system = ActorSystem::builder(store).build();
    assert_eq!(system.list_actors("", None, 10).await.unwrap().items.len(), 1);
}
