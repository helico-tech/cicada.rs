//! Request validation and status codes of the HTTP API, on virtual time.

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use cicada::api::problem_types as pt;
use cicada_axum::{ApiConfig, actor_api, actor_api_with};
use common::*;
use serde_json::{Value, json};

async fn app() -> Router {
    actor_api(node(memory_store()).await)
}

fn problem_type(reply: &Reply) -> Value {
    reply.json()["type"].clone()
}

fn assert_status(reply: &Reply, status: u16, problem_type_: &str) {
    assert_eq!(reply.status.as_u16(), status, "{}", reply.text);
    assert_eq!(problem_type(reply), problem_type_, "{}", reply.text);
}

#[tokio::test(start_paused = true)]
async fn delivery_times_beyond_the_supported_range_are_refused_with_400() {
    let app = app().await;
    put(&app, "/actors/counter/t", r#"{"state": {}}"#).await;
    for when in [
        r#""deliverAt": "+300000-01-01T00:00:00Z""#,
        r#""deliverAt": "9999-12-31T23:59:59.9999999Z""#,
        r#""delay": 9223372036854775807"#,
        r#""delay": "PT2562047788015215H""#,
        r#""delay": "INFINITE""#,
    ] {
        let reply =
            post(&app, "/actors/counter/t/messages", &format!(r#"{{"type": "Add", "payload": {{"n": 1}}, {when}}}"#))
                .await;
        assert_status(&reply, 400, pt::BAD_REQUEST);
    }
    assert!(items(&get_json(&app, "/actors/counter/t/mailbox").await).is_empty());
}

#[tokio::test(start_paused = true)]
async fn unknown_keys_in_request_envelopes_are_refused_with_400_while_payloads_stay_lenient() {
    let app = app().await;
    assert_status(&put(&app, "/actors/counter/k", r#"{"stat": {"total": 1}}"#).await, 400, pt::BAD_REQUEST);
    assert_eq!(
        put(&app, "/actors/counter/k", r#"{"state": {"total": 1, "unknown": true}}"#).await.status,
        StatusCode::CREATED
    );
    let path = "/actors/counter/k/messages";
    assert_status(&post(&app, path, r#"{"type": "Add", "paylaod": {"n": 1}}"#).await, 400, pt::BAD_REQUEST);
    assert_status(
        &post(&app, path, r#"{"type": "Add", "payload": {"n": 1}, "delayMs": 500}"#).await,
        400,
        pt::BAD_REQUEST,
    );
    let ok = post(&app, path, r#"{"type": "Add", "payload": {"n": 1, "extra": 2}}"#).await;
    assert_eq!(ok.status, StatusCode::ACCEPTED, "{}", ok.text);
}

#[tokio::test(start_paused = true)]
async fn a_repeated_id_is_answered_as_a_duplicate_first_and_a_reused_id_for_another_message_conflicts() {
    let app = app().await;
    put(&app, "/actors/counter/d", r#"{"state": {}}"#).await;
    put(&app, "/actors/counter/e", r#"{"state": {}}"#).await;
    let id = unique();
    let body = format!(r#"{{"id": "{id}", "type": "Add", "payload": {{"n": 1}}, "delay": "PT1H"}}"#);
    assert_eq!(post(&app, "/actors/counter/d/messages", &body).await.status, StatusCode::ACCEPTED);
    // The same send retried: 200 even though this request would be refused now (as when the first attempt went
    // through a node that does not host the type and accepted the payload undecoded).
    let retried = post(
        &app,
        "/actors/counter/d/messages",
        &format!(r#"{{"id": "{id}", "type": "Add", "payload": {{"n": "not a number"}}, "delay": "PT1H"}}"#),
    )
    .await;
    assert_eq!(retried.status, StatusCode::OK, "{}", retried.text);
    // The id reused for a different message: 409.
    assert_status(&post(&app, "/actors/counter/e/messages", &body).await, 409, pt::MESSAGE_ID_CONFLICT);
    let other_type = format!(r#"{{"id": "{id}", "type": "Fail", "payload": {{"reason": "x"}}}}"#);
    let conflict = post(&app, "/actors/counter/d/messages", &other_type).await;
    assert_status(&conflict, 409, pt::MESSAGE_ID_CONFLICT);
    assert_eq!(
        conflict.json()["detail"],
        format!("Message id {id} was already accepted for a Add message to counter/d")
    );
}

#[tokio::test(start_paused = true)]
async fn oversized_or_nul_containing_envelope_fields_are_refused_with_400() {
    let app = app().await;
    put(&app, "/actors/counter/n", r#"{"state": {}}"#).await;
    let too_long = "x".repeat(300);
    for field in [
        format!(r#""id": "{too_long}""#),
        r#""correlationId": "a\u0000b""#.to_owned(),
        r#""headers": {"trace": "a\u0000b"}"#.to_owned(),
        format!(r#""type": "{too_long}""#),
    ] {
        let reply =
            post(&app, "/actors/counter/n/messages", &format!(r#"{{"type": "Add", "payload": {{"n": 1}}, {field}}}"#))
                .await;
        assert_status(&reply, 400, pt::BAD_REQUEST);
    }
}

#[tokio::test(start_paused = true)]
async fn an_invalid_retry_policy_is_refused_with_400_valid_forms_are_accepted() {
    let app = app().await;
    put(&app, "/actors/counter/rp", r#"{"state": {}}"#).await;
    for retry in [
        r#"{"maxAttempts": 0, "backoff": {"kind": "fixed", "delay": "PT1S"}}"#,
        r#"{"maxAttempts": 101, "backoff": {"kind": "fixed", "delay": "PT1S"}}"#,
        r#"{"maxAttempts": 3}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "linear", "delay": "PT1S"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT0S"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "-PT1S"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "P400000D"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "soon"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "PT1S", "factor": 0.5, "max": "PT1M"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "PT1M", "max": "PT1S"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT1S"}, "jitter": 0.1}"#,
        // Unknown keys inside the backoff are refused as well.
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT1S", "jitter": 0.1}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "PT1S", "max": "PT1M", "cap": "PT1H"}}"#,
        r#""three times""#,
        // A duration is an ISO-8601 string: not the short form, not a number of milliseconds.
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "5s"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": 1500}}"#,
    ] {
        let reply = post(
            &app,
            "/actors/counter/rp/messages",
            &format!(r#"{{"type": "Add", "payload": {{"n": 1}}, "retry": {retry}}}"#),
        )
        .await;
        assert_status(&reply, 400, pt::BAD_REQUEST);
    }
    for retry in [
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT5S"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT1.5S"}}"#,
        r#"{"maxAttempts": 100, "backoff": {"kind": "exponential", "initial": "PT1S", "max": "PT1M"}}"#,
        "null",
    ] {
        let reply = post(
            &app,
            "/actors/counter/rp/messages",
            &format!(r#"{{"type": "Add", "payload": {{"n": 1}}, "retry": {retry}}}"#),
        )
        .await;
        assert_eq!(reply.status, StatusCode::ACCEPTED, "{retry}: {}", reply.text);
    }
}

#[tokio::test(start_paused = true)]
async fn an_invalid_retention_policy_is_refused_with_400_valid_forms_are_accepted_and_served() {
    let app = app().await;
    put(&app, "/actors/counter/ret", r#"{"state": {}}"#).await;
    let path = "/actors/counter/ret/messages";
    for retention in [
        r#"{"done": {"kind": "forever"}}"#,
        r#"{"done": {"kind": "last", "count": 0}}"#,
        r#"{"done": {"kind": "last", "count": 10001}}"#,
        r#"{"failed": {"kind": "for", "age": "PT0S"}}"#,
        r#"{"failed": {"kind": "for", "age": "soon"}}"#,
        r#"{"failed": {"kind": "for"}}"#,
        r#"{"failed": {"kind": "for", "age": 60000}}"#,
        r#"{"failed": {"kind": "for", "age": "60s"}}"#,
        r#"{"cancelled": {"kind": "remove"}}"#,
        r#""remove""#,
    ] {
        let reply =
            post(&app, path, &format!(r#"{{"type": "Add", "payload": {{"n": 1}}, "retention": {retention}}}"#)).await;
        assert_status(&reply, 400, pt::BAD_REQUEST);
    }
    for retention in [
        r#"{"done": {"kind": "remove"}}"#,
        r#"{"failed": {"kind": "for", "age": "P30D"}}"#,
        r#"{"done": {"kind": "last", "count": 10}, "failed": {"kind": "for", "age": "PT1M"}}"#,
        "{}",
        "null",
    ] {
        let body = format!(r#"{{"type": "Add", "payload": {{"n": 1}}, "delay": "PT1H", "retention": {retention}}}"#);
        let reply = post(&app, path, &body).await;
        assert_eq!(reply.status, StatusCode::ACCEPTED, "{retention}: {}", reply.text);
    }
    let page = get_json(&app, "/actors/counter/ret/mailbox").await;
    let served: Vec<&Value> = items(&page).iter().map(|m| &m["retention"]).collect();
    assert_eq!(served[2], &json!({"done": {"kind": "last", "count": 10}, "failed": {"kind": "for", "age": "PT1M"}}));
    assert_eq!(served[4], &Value::Null);
}

#[tokio::test(start_paused = true)]
async fn a_message_removed_on_success_answers_404_but_stays_a_duplicate_and_the_cause_of_what_it_sent() {
    let app = app().await;
    for c in ["from", "to"] {
        put(&app, &format!("/actors/counter/{c}"), r#"{"state": {}}"#).await;
    }
    // The receiver is busy, so the message sent to it stays in its mailbox.
    let gate = unique();
    let blocked = send_id(
        &app,
        "/actors/counter/to/messages",
        &format!(r#"{{"type": "Block", "payload": {{"gate": "{gate}"}}}}"#),
    )
    .await;
    eventually(async || get_json(&app, &format!("/messages/{blocked}")).await["status"] == "running").await;
    let id = unique();
    let sent = post(
        &app,
        "/actors/counter/from/messages",
        &format!(r#"{{"id": "{id}", "type": "Forward", "payload": {{"to": ["to"], "n": 1}}, "retention": {{"done": {{"kind": "remove"}}}}}}"#),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED);
    eventually(async || get(&app, &format!("/messages/{id}")).await.status == StatusCode::NOT_FOUND).await;
    assert_status(&get(&app, &format!("/messages/{id}")).await, 404, pt::NO_SUCH_MESSAGE);
    // The child message is kept (no policy) and still names the removed message as its cause.
    let mailbox = get_json(&app, "/actors/counter/to/mailbox").await;
    let sent: Vec<&Value> = items(&mailbox).iter().filter(|m| m["type"] == "Add").collect();
    let [child] = sent.as_slice() else { panic!("one message from the removed one: {mailbox}") };
    assert_eq!(child["causedBy"], id.as_str());
    let child_id = child["id"].as_str().unwrap().to_owned();

    gates::open(&gate);
    eventually(async || get_json(&app, "/actors/counter/to").await["state"]["total"] == 1).await;
    let again = post(
        &app,
        "/actors/counter/from/messages",
        &format!(r#"{{"id": "{id}", "type": "Forward", "payload": {{"to": ["to"], "n": 1}}}}"#),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK, "still a duplicate");

    assert_eq!(get_json(&app, "/actors/counter/to").await["state"]["total"], 1, "handled once");
    let child = get_json(&app, &format!("/messages/{child_id}")).await;
    assert_eq!(child["status"], "done");
    assert_eq!(child["message"]["causedBy"], id.as_str());
}

#[tokio::test(start_paused = true)]
async fn explicit_nulls_for_every_optional_send_field_are_accepted() {
    let app = app().await;
    put(&app, "/actors/counter/nulls", r#"{"state": {}}"#).await;
    let reply = post(
        &app,
        "/actors/counter/nulls/messages",
        r#"{"id": null, "type": "Add", "payload": {"n": 1}, "deliverAt": null, "delay": null, "correlationId": null, "retry": null, "retention": null}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::ACCEPTED, "{}", reply.text);
}

#[tokio::test(start_paused = true)]
async fn an_invalid_duration_in_a_backoff_or_as_the_delay_is_refused_with_400() {
    let app = app().await;
    put(&app, "/actors/counter/dur", r#"{"state": {}}"#).await;
    for retry in [
        r#"{"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "soon", "max": "PT1M"}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "PT1S", "max": 1.5}}"#,
        r#"{"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": {"s": 1}}}"#,
    ] {
        let reply = post(
            &app,
            "/actors/counter/dur/messages",
            &format!(r#"{{"type": "Add", "payload": {{"n": 1}}, "retry": {retry}}}"#),
        )
        .await;
        assert_status(&reply, 400, pt::BAD_REQUEST);
    }
    let reply =
        post(&app, "/actors/counter/dur/messages", r#"{"type": "Add", "payload": {"n": 1}, "delay": "soon"}"#).await;
    assert_status(&reply, 400, pt::BAD_REQUEST);
    assert!(items(&get_json(&app, "/actors/counter/dur/mailbox").await).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_duplicate_send_with_a_different_retry_policy_is_200_and_keeps_the_first_policy() {
    let app = app().await;
    put(&app, "/actors/counter/dup", r#"{"state": {}}"#).await;
    let id = unique();
    let first = r#"{"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT1H"}}"#;
    let second = r#"{"maxAttempts": 5, "backoff": {"kind": "fixed", "delay": "PT1S"}}"#;
    let sent = post(
        &app,
        "/actors/counter/dup/messages",
        &format!(r#"{{"id": "{id}", "type": "Add", "payload": {{"n": 1}}, "delay": "PT1H", "retry": {first}}}"#),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED);
    let again = post(
        &app,
        "/actors/counter/dup/messages",
        &format!(r#"{{"id": "{id}", "type": "Add", "payload": {{"n": 1}}, "retry": {second}}}"#),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    let envelope = &get_json(&app, &format!("/messages/{id}")).await["message"];
    assert_eq!(envelope["retry"], serde_json::from_str::<Value>(first).unwrap());
}

#[tokio::test(start_paused = true)]
async fn a_retried_message_is_pending_with_its_failures_last_error_and_policy_listed_and_cancellable() {
    let app = actor_api(node_with(memory_store(), |b| b.host(CounterActor)).await);
    put(&app, "/actors/counter/r", r#"{"state": {}}"#).await;
    let original = unique();
    let retry = r#"{"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT1H"}}"#;
    post(
        &app,
        "/actors/counter/r/messages",
        &format!(r#"{{"id": "{original}", "type": "Fail", "payload": {{"reason": "flaky"}}, "retry": {retry}}}"#),
    )
    .await;
    eventually(async || get_json(&app, &format!("/messages/{original}")).await["failures"] == 1).await;
    let status = get_json(&app, &format!("/messages/{original}")).await;
    assert_eq!(status["status"], "pending");
    let last_error = status["lastError"].as_str().unwrap().to_owned();
    assert!(last_error.contains("flaky"), "{last_error}");
    assert!(status.get("retriedAs").is_none());
    let envelope = &status["message"];
    assert_eq!(envelope["failures"], 1);
    assert_eq!(envelope["lastError"], last_error.as_str());
    assert_eq!(envelope["attempt"], 1);
    assert_eq!(envelope["retry"], serde_json::from_str::<Value>(retry).unwrap());

    let page = get_json(&app, "/actors/counter/r/mailbox").await;
    let [listed] = items(&page).as_slice() else { panic!("one pending message: {page}") };
    assert_eq!(listed["id"], original.as_str());
    assert_eq!(listed["failures"], 1);
    assert_eq!(listed["lastError"], last_error.as_str());
    assert_eq!(listed["retry"], serde_json::from_str::<Value>(retry).unwrap());
    assert!(items(&get_json(&app, "/outcomes?status=failed").await).is_empty());

    assert_eq!(post(&app, &format!("/messages/{original}/cancel"), "").await.status, StatusCode::NO_CONTENT);
    let cancelled = get_json(&app, &format!("/messages/{original}")).await;
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["failures"], 1);
}

#[tokio::test(start_paused = true)]
async fn a_request_after_the_store_is_closed_answers_503() {
    // A panic and corrupt data as `500` are unit tests of the error mapping (`problem.rs`, `handlers.rs`).
    let store = memory_store();
    let app = actor_api(node(store.clone()).await);
    assert_eq!(get(&app, "/actors/counter/x").await.status, StatusCode::NOT_FOUND);
    store.close().await;
    let unavailable = get(&app, "/actors/counter/x").await;
    assert_status(&unavailable, 503, pt::STORE_UNAVAILABLE);
    assert_eq!(unavailable.json()["instance"], "/actors/counter/x");
}

#[tokio::test(start_paused = true)]
async fn outcomes_since_an_instant_beyond_the_supported_range_is_a_400_on_every_backend() {
    let app = app().await;
    for since in ["+300000-01-01T00:00:00Z", "-300000-01-01T00:00:00Z", "9999-12-31T23:59:59.9999999Z"] {
        assert_status(&get(&app, &format!("/outcomes?since={}", enc(since))).await, 400, pt::BAD_REQUEST);
    }
    assert_eq!(get(&app, "/outcomes?since=9999-12-31T00:00:00Z").await.status, StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn request_bodies_above_the_limit_are_refused_with_413() {
    let system = node_with(memory_store(), |b| b.host(CounterActor)).await;
    let app = actor_api_with(system, ApiConfig { max_request_body_bytes: 200 });
    assert_eq!(put(&app, "/actors/counter/b", r#"{"state": {}}"#).await.status, StatusCode::CREATED);
    let big = format!(r#"{{"type": "Add", "payload": {{"n": 1, "padding": "{}"}}}}"#, "x".repeat(300));
    assert_status(&post(&app, "/actors/counter/b/messages", &big).await, 413, pt::REQUEST_TOO_LARGE);
    let big_state = format!(r#"{{"state": {{"padding": "{}"}}}}"#, "x".repeat(300));
    assert_status(&put(&app, "/actors/counter/c", &big_state).await, 413, pt::REQUEST_TOO_LARGE);
    // Without Content-Length (streamed), reading stops past the limit.
    let chunks: Vec<Result<String, std::io::Error>> =
        big.as_bytes().chunks(50).map(|c| Ok(String::from_utf8(c.to_vec()).unwrap())).collect();
    let streamed = Request::builder()
        .method(Method::POST)
        .uri("/actors/counter/b/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(futures::stream::iter(chunks)))
        .unwrap();
    assert!(streamed.headers().get(header::CONTENT_LENGTH).is_none());
    assert_status(&request(&app, streamed).await, 413, pt::REQUEST_TOO_LARGE);
    // A lying Content-Length is refused before reading.
    let declared = Request::builder()
        .method(Method::POST)
        .uri("/actors/counter/b/messages")
        .header(header::CONTENT_LENGTH, "201")
        .body(Body::from(r#"{"type": "Add", "payload": {"n": 1}}"#))
        .unwrap();
    assert_status(&request(&app, declared).await, 413, pt::REQUEST_TOO_LARGE);
    let small = post(&app, "/actors/counter/b/messages", r#"{"type": "Add", "payload": {"n": 1}}"#).await;
    assert_eq!(small.status, StatusCode::ACCEPTED);
}

#[test]
#[should_panic(expected = "max_request_body_bytes must be positive")]
fn a_zero_body_limit_is_refused() {
    let system = cicada::ActorSystem::builder(memory_store()).build();
    let _ = actor_api_with(system, ApiConfig { max_request_body_bytes: 0 });
}

#[tokio::test(start_paused = true)]
async fn a_nul_character_in_a_payload_or_state_is_a_400_on_every_backend() {
    let app = app().await;
    assert_status(&put(&app, "/actors/counter/z", r#"{"state": {"note": "a\u0000b"}}"#).await, 400, pt::BAD_REQUEST);
    put(&app, "/actors/counter/z", r#"{"state": {}}"#).await;
    let reply = post(&app, "/actors/counter/z/messages", r#"{"type": "Add", "payload": {"n": 1, "x\u0000": 1}}"#).await;
    assert_status(&reply, 400, pt::BAD_REQUEST);
}

#[tokio::test(start_paused = true)]
async fn malformed_queries_and_bodies_are_problems() {
    let app = app().await;
    put(&app, "/actors/counter/q", r#"{"state": {}}"#).await;
    let latin1 = Request::builder()
        .method(Method::POST)
        .uri("/actors/counter/q/messages")
        .body(Body::from(vec![b'{', 0xE9, b'}']))
        .unwrap();
    assert_status(&request(&app, latin1).await, 400, pt::BAD_REQUEST);
    assert_status(&post(&app, "/actors/counter/q/messages", r#"["Add"]"#).await, 400, pt::BAD_REQUEST);
}
