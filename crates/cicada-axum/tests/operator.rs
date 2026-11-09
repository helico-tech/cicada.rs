//! The operator routes of the HTTP contract: nodes, running turns, the mailbox, and message and bulk actions.

mod common;

use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use cicada::api::problem_types as pt;
use cicada_axum::{actor_api, actor_api_read};
use common::*;
use serde_json::json;

async fn system_and_app() -> (cicada::ActorSystem, Router) {
    let system = node(memory_store()).await;
    (system.clone(), actor_api(system))
}

async fn add(app: &Router, actor: &str, delay: &str) -> String {
    send_id(
        app,
        &format!("/actors/counter/{actor}/messages"),
        &format!(r#"{{"type": "Add", "payload": {{"n": 1}}, "delay": "{delay}"}}"#),
    )
    .await
}

async fn block(app: &Router, actor: &str, gate: &str) -> String {
    let id = send_id(
        app,
        &format!("/actors/counter/{actor}/messages"),
        &format!(r#"{{"type": "Block", "payload": {{"gate": "{gate}"}}}}"#),
    )
    .await;
    eventually(async || get_json(app, &format!("/messages/{id}")).await["status"] == "running").await;
    id
}

// ---- nodes ----

#[tokio::test(start_paused = true)]
async fn nodes_lists_this_node_alive_and_a_detail_lists_its_running_turns() {
    let (system, app) = system_and_app().await;
    let node_id = system.node_id().to_string();
    let listed = eventually_json(&app, "/nodes", |v| !v["items"].as_array().unwrap().is_empty()).await;
    assert_eq!(listed["items"][0]["id"], node_id.as_str());
    assert_eq!(listed["items"][0]["status"], "alive");
    assert_eq!(listed["items"][0]["hosted"], json!(["counter", "ping"]));
    assert_eq!(listed["items"][0]["runningTurns"], 0);
    assert!(listed["items"][0]["lastSeen"].is_string() && listed["items"][0]["startedAt"].is_string());

    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let gate = unique();
    let running = block(&app, "a", &gate).await;

    let detail = get_json(&app, &format!("/nodes/{node_id}")).await;
    assert_eq!(detail["id"], node_id.as_str());
    assert_eq!(detail["status"], "alive");
    assert_eq!(detail["runningTurns"], 1);
    assert_eq!(detail["turns"][0]["actor"], "counter/a");
    assert_eq!(detail["turns"][0]["messageId"], running.as_str());
    assert_eq!(detail["turns"][0]["cancelRequested"], false);

    assert_problem(&get(&app, "/nodes/nope").await, 404, pt::NO_SUCH_NODE);
    gates::open(&gate);
}

async fn eventually_json(app: &Router, uri: &str, ok: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
    for _ in 0..500 {
        let value = get_json(app, uri).await;
        if ok(&value) {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{uri} never matched");
}

// ---- turns ----

#[tokio::test(start_paused = true)]
async fn turns_lists_running_turns_filtered_and_one_actors_turn_can_be_read_and_cancelled() {
    let (_system, app) = system_and_app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    put(&app, "/actors/counter/b", r#"{"state": {}}"#).await;
    let gate = unique();
    let running = block(&app, "a", &gate).await;
    let other = block(&app, "b", &gate).await;

    let all = get_json(&app, "/turns").await;
    assert_eq!(items(&all).len(), 2);
    assert_eq!(items(&get_json(&app, "/turns?prefix=counter/a").await).len(), 1);
    assert_eq!(items(&get_json(&app, "/turns?limit=1").await).len(), 1);
    let turn = get_json(&app, "/actors/counter/a/turn").await;
    assert_eq!(turn["actor"], "counter/a");
    assert_eq!(turn["messageId"], running.as_str());
    let node = turn["node"].as_str().unwrap().to_owned();
    assert_eq!(items(&get_json(&app, &format!("/turns?node={node}")).await).len(), 2);
    assert_eq!(items(&get_json(&app, "/turns?node=nope").await).len(), 0);

    assert_problem(&get(&app, "/actors/counter/nope/turn").await, 404, pt::NO_RUNNING_TURN);
    assert_problem(&post(&app, "/actors/counter/nope/turn/cancel", "").await, 404, pt::NO_RUNNING_TURN);

    let cancel = post(&app, "/actors/counter/a/turn/cancel", "").await;
    assert_eq!(cancel.status, StatusCode::ACCEPTED);
    assert!(cancel.text.is_empty());
    assert_eq!(get_json(&app, "/actors/counter/a/turn").await["cancelRequested"], true);
    eventually(async || get_json(&app, &format!("/messages/{running}")).await["status"] == "cancelled").await;
    assert_problem(&get(&app, "/actors/counter/a/turn").await, 404, pt::NO_RUNNING_TURN);
    assert_eq!(get_json(&app, &format!("/messages/{other}")).await["status"], "running");
    gates::open(&gate);
}

// ---- mailbox ----

#[tokio::test(start_paused = true)]
async fn the_mailbox_marks_running_and_can_be_paused_resumed_and_purged() {
    let (_system, app) = system_and_app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    assert_eq!(get_json(&app, "/actors/counter/a").await["paused"], false);

    assert_problem(&post(&app, "/actors/counter/nope/mailbox/pause", "").await, 404, pt::NO_SUCH_INSTANCE);
    assert_problem(&post(&app, "/actors/counter/nope/mailbox/resume", "").await, 404, pt::NO_SUCH_INSTANCE);
    assert_problem(&post(&app, "/actors/counter/nope/mailbox/purge", "").await, 404, pt::NO_SUCH_INSTANCE);

    let gate = unique();
    let running = block(&app, "a", &gate).await;
    let waiting = add(&app, "a", "PT0S").await;
    let page = get_json(&app, "/actors/counter/a/mailbox").await;
    assert_eq!(items(&page)[0]["id"], running.as_str());
    assert_eq!(items(&page)[0]["state"], "running");
    assert_eq!(items(&page)[1]["id"], waiting.as_str());
    assert_eq!(items(&page)[1]["state"], "pending");

    assert_eq!(post(&app, "/actors/counter/a/mailbox/pause", "").await.status, StatusCode::NO_CONTENT);
    assert_eq!(get_json(&app, "/actors/counter/a").await["paused"], true);
    gates::open(&gate);
    eventually(async || get_json(&app, &format!("/messages/{running}")).await["status"] == "done").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(get_json(&app, &format!("/messages/{waiting}")).await["status"], "pending", "paused: no new turn");

    let purged = post(&app, "/actors/counter/a/mailbox/purge", "").await;
    assert_eq!(purged.status, StatusCode::OK);
    assert_eq!(purged.json(), json!({"purged": 1}));
    assert_eq!(get_json(&app, &format!("/messages/{waiting}")).await["status"], "purged");
    assert!(items(&get_json(&app, "/actors/counter/a/mailbox").await).is_empty());
    assert_eq!(get_json(&app, "/actors/counter/a").await["version"], 0, "the actor stays");

    assert_eq!(post(&app, "/actors/counter/a/mailbox/resume", "").await.status, StatusCode::NO_CONTENT);
    assert_eq!(get_json(&app, "/actors/counter/a").await["paused"], false);
    let next = add(&app, "a", "PT0S").await;
    eventually(async || get_json(&app, &format!("/messages/{next}")).await["status"] == "done").await;
}

#[tokio::test(start_paused = true)]
async fn the_old_mailbox_listing_is_gone() {
    let (_, app) = system_and_app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    assert_eq!(get(&app, "/actors/counter/a/messages").await.status, StatusCode::METHOD_NOT_ALLOWED);
}

// ---- message actions ----

#[tokio::test(start_paused = true)]
async fn retry_answers_202_with_a_new_message_and_the_refusals() {
    let (_, app) = system_and_app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let body = r#"{"type": "Fail", "payload": {"reason": "no"}, "correlationId": "corr"}"#;
    let failed = send_id(&app, "/actors/counter/a/messages", body).await;
    eventually(async || get_json(&app, &format!("/messages/{failed}")).await["status"] == "failed").await;

    let retried = post(&app, &format!("/messages/{failed}/retry"), "").await;
    assert_eq!(retried.status, StatusCode::ACCEPTED, "{}", retried.text);
    let new_id = retried.json()["id"].as_str().unwrap().to_owned();
    assert_ne!(new_id, failed);
    let status = get_json(&app, &format!("/messages/{new_id}")).await;
    assert_eq!(status["message"]["causedBy"], failed.as_str());
    assert_eq!(status["message"]["correlationId"], "corr");

    let pending = add(&app, "a", "PT1H").await;
    assert_problem(&post(&app, &format!("/messages/{pending}/retry"), "").await, 409, pt::MESSAGE_NOT_RETRYABLE);
    let done = add(&app, "a", "PT0S").await;
    eventually(async || get_json(&app, &format!("/messages/{done}")).await["status"] == "done").await;
    assert_problem(&post(&app, &format!("/messages/{done}/retry"), "").await, 409, pt::MESSAGE_NOT_RETRYABLE);
    assert_problem(&post(&app, &format!("/messages/{}/retry", unique()), "").await, 404, pt::NO_SUCH_MESSAGE);

    assert_eq!(delete(&app, "/actors/counter/a").await.status, StatusCode::NO_CONTENT);
    assert_problem(&post(&app, &format!("/messages/{failed}/retry"), "").await, 404, pt::NO_SUCH_INSTANCE);
}

#[tokio::test(start_paused = true)]
async fn deliver_now_answers_200_with_the_message_and_409_for_running_or_finished() {
    let (_, app) = system_and_app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let later = add(&app, "a", "PT1H").await;
    let reply = post(&app, &format!("/messages/{later}/deliver-now"), "").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json()["id"], later.as_str());
    eventually(async || get_json(&app, &format!("/messages/{later}")).await["status"] == "done").await;
    assert_problem(&post(&app, &format!("/messages/{later}/deliver-now"), "").await, 409, pt::MESSAGE_FINISHED);
    assert_problem(&post(&app, &format!("/messages/{}/deliver-now", unique()), "").await, 404, pt::NO_SUCH_MESSAGE);
    let gate = unique();
    let running = block(&app, "a", &gate).await;
    assert_problem(&post(&app, &format!("/messages/{running}/deliver-now"), "").await, 409, pt::MESSAGE_RUNNING);
    gates::open(&gate);
}

#[tokio::test(start_paused = true)]
async fn delete_of_a_message_is_gone_and_cancel_is_a_post() {
    let (_, app) = system_and_app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let later = add(&app, "a", "PT1H").await;
    assert_eq!(delete(&app, &format!("/messages/{later}")).await.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(post(&app, &format!("/messages/{later}/cancel"), "").await.status, StatusCode::NO_CONTENT);
}

// ---- bulk actions ----

#[tokio::test(start_paused = true)]
async fn bulk_cancel_by_filter_or_ids_answers_affected_and_more() {
    let (_, app) = system_and_app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let ids = [add(&app, "a", "PT1H").await, add(&app, "a", "PT1H").await];

    let by_ids = post(&app, "/messages/bulk-cancel", &json!({"ids": [ids[0].clone()]}).to_string()).await;
    assert_eq!(by_ids.status, StatusCode::OK, "{}", by_ids.text);
    assert_eq!(by_ids.json(), json!({"affected": 1, "more": false}));

    let by_filter = post(&app, "/messages/bulk-cancel", r#"{"prefix": "counter/", "messageType": "Add"}"#).await;
    assert_eq!(by_filter.json(), json!({"affected": 1, "more": false}));
    assert!(items(&get_json(&app, "/actors/counter/a/mailbox").await).is_empty());
}

#[tokio::test(start_paused = true)]
async fn bulk_bodies_need_exactly_ids_or_a_filter() {
    let (_, app) = system_and_app().await;
    for bad in
        ["", "{}", r#"{"ids": ["x"], "prefix": "p"}"#, r#"{"ids": ["x"], "messageType": "T"}"#, r#"{"unknown": 1}"#]
    {
        assert_problem(&post(&app, "/messages/bulk-cancel", bad).await, 400, pt::BAD_REQUEST);
        assert_problem(&post(&app, "/messages/bulk-retry", bad).await, 400, pt::BAD_REQUEST);
    }
    let ids: Vec<String> = (0..1001).map(|i| format!("m{i}")).collect();
    let too_many = post(&app, "/messages/bulk-cancel", &json!({"ids": ids}).to_string()).await;
    assert_problem(&too_many, 400, pt::BAD_REQUEST);
}

#[tokio::test(start_paused = true)]
async fn bulk_retry_skips_ineligible_messages_without_counting_them() {
    let (_, app) = system_and_app().await;
    put(&app, "/actors/counter/a", r#"{"state": {}}"#).await;
    let failed = send_id(&app, "/actors/counter/a/messages", r#"{"type": "Fail", "payload": {"reason": "no"}}"#).await;
    eventually(async || get_json(&app, &format!("/messages/{failed}")).await["status"] == "failed").await;
    let pending = add(&app, "a", "PT1H").await;

    let reply = post(&app, "/messages/bulk-retry", &json!({"ids": [failed, pending]}).to_string()).await;
    assert_eq!(reply.json(), json!({"affected": 1, "more": false}));
    let by_filter = post(&app, "/messages/bulk-retry", r#"{"prefix": "counter/"}"#).await;
    assert_eq!(by_filter.json(), json!({"affected": 0, "more": false}), "already retried");
}

#[tokio::test(start_paused = true)]
async fn the_read_router_serves_the_read_routes() {
    let system = node(memory_store()).await;
    let app = actor_api_read(system);
    eventually_json(&app, "/nodes", |v| !v["items"].as_array().unwrap().is_empty()).await;
    assert!(items(&get_json(&app, "/turns").await).is_empty());
}
