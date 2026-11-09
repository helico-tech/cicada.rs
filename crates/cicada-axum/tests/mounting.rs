//! Mounting the routers into a host application: next to service routes, behind the host's authentication, under a
//! prefix, a public readiness check, several nodes on one store and restarts, on virtual time.

mod common;

use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post as post_route;
use cicada::api::problem_types as pt;
use cicada::store::NodeId;
use cicada::{ActorSystem, SendResult};
use cicada_axum::{PROBLEM_JSON, actor_api, actor_health};
use common::*;
use tokio::time::Instant;

/// The host's authentication: requests without `authorization` equal to `expected` get `401`.
fn authenticated(router: Router, expected: &'static str) -> Router {
    router.layer(middleware::from_fn(move |req: Request, next: Next| async move {
        if req.headers().get(header::AUTHORIZATION).is_some_and(|v| v == expected) {
            next.run(req).await
        } else {
            (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic"))]).into_response()
        }
    }))
}

const BASIC: &str = "Basic c3ZjOnNlY3JldA=="; // svc:secret
const BEARER: &str = "Bearer t0ken";

async fn get_with(app: &Router, uri: &str, authorization: &str) -> Reply {
    let req = axum::http::Request::builder()
        .uri(uri)
        .header(header::AUTHORIZATION, authorization)
        .body(axum::body::Body::empty())
        .unwrap();
    request(app, req).await
}

#[tokio::test(start_paused = true)]
async fn the_system_is_the_in_process_sdk_next_to_the_api() {
    let system = node_with(memory_store(), |b| b.host(CounterActor).node_id(NodeId::new("node-x").unwrap())).await;
    async fn bump(State(system): State<ActorSystem>, Path(id): Path<String>) -> Response {
        // Service code sends without the HTTP hop.
        match system.send(&Counter { id }, CounterMsg::Add { n: 10 }).await {
            Ok(SendResult::Accepted(_)) => "ok".into_response(),
            other => format!("{other:?}").into_response(),
        }
    }
    let app =
        Router::new().route("/bump/{id}", post_route(bump)).with_state(system.clone()).merge(actor_api(system.clone()));

    assert_eq!(get_json(&app, "/health").await["node"], "node-x");
    system.create(&Counter { id: "a".into() }, &CounterState::default()).await.unwrap();
    assert!(matches!(
        system.send(&Counter { id: "a".into() }, CounterMsg::Add { n: 2 }).await,
        Ok(SendResult::Accepted(_))
    ));
    assert_eq!(post(&app, "/bump/a", "").await.text, "ok");
    eventually(async || get_json(&app, "/actors/counter/a").await["version"] == 2).await;
    assert_eq!(get_json(&app, "/actors/counter/a").await["state"]["total"], 12);
    system.stop().await;
    assert_problem(&get(&app, "/health").await, 503, pt::UNHEALTHY);
}

#[tokio::test(start_paused = true)]
async fn a_non_hosting_node_accepts_any_payload_which_then_ends_as_the_unhandled_policy_says() {
    let store = memory_store();
    let a = actor_api(node_with(store.clone(), |b| b.host(CounterActor)).await);
    let b = actor_api(node_with(store, |b| b).await);
    // State is only decoded where the type is hosted.
    assert_eq!(
        put(&a, "/actors/counter/x", r#"{"state": {"total": "x"}}"#).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(put(&b, "/actors/counter/c", r#"{"state": {}}"#).await.status, StatusCode::CREATED);

    let bad = r#"{"type": "Add", "payload": {"n": "not a number"}}"#;
    assert_eq!(post(&a, "/actors/counter/c/messages", bad).await.status, StatusCode::UNPROCESSABLE_ENTITY);
    let id = send_id(&b, "/actors/counter/c/messages", bad).await;

    // The type name is still checked by any node, through the TypeRegistry.
    assert_eq!(
        post(&b, "/actors/counter/c/messages", r#"{"type": "Nope"}"#).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    eventually(async || get_json(&b, &format!("/messages/{id}")).await["status"] == "failed").await;
    assert!(!get_json(&b, &format!("/messages/{id}")).await["error"].as_str().unwrap().trim().is_empty());

    // A good payload sent through the send-only node is handled by the hosting node.
    post(&b, "/actors/counter/c/messages", r#"{"type": "Add", "payload": {"n": 3}}"#).await;
    eventually(async || get_json(&a, "/actors/counter/c").await["state"]["total"] == 3).await;
    let types = get_json(&b, "/types").await;
    assert_eq!(items(&types).iter().map(|t| t["template"].clone()).collect::<Vec<_>>(), ["actor://counter/{id}"]);
    assert_eq!(get(&b, "/health").await.status, StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn the_api_works_behind_the_hosts_basic_authentication() {
    let app = authenticated(actor_api(node(memory_store()).await), BASIC);
    assert_eq!(get(&app, "/health").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(put(&app, "/actors/counter/a", r#"{"state": {}}"#).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(get_with(&app, "/types", "Basic c3ZjOndyb25n").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(get_with(&app, "/health", BASIC).await.status, StatusCode::OK);
    assert_eq!(get_with(&app, "/types", BASIC).await.status, StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn the_api_works_behind_bearer_authentication_under_a_route_prefix() {
    let app = Router::new().nest("/internal", authenticated(actor_api(node(memory_store()).await), BEARER));
    assert_eq!(get(&app, "/internal/health").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(get_with(&app, "/internal/health", BEARER).await.status, StatusCode::OK);
    assert_eq!(put(&app, "/internal/actors/counter/a", r#"{"state": {}}"#).await.status, StatusCode::UNAUTHORIZED);
    // Problems name the request as the client sent it, prefix included; mailboxes resolve below the prefix.
    let missing = get_with(&app, "/internal/actors/counter/zz/messages?limit=5", BEARER).await;
    assert_problem(&missing, 404, pt::NO_SUCH_INSTANCE);
    assert_eq!(missing.json()["instance"], "/internal/actors/counter/zz/messages?limit=5");
    assert_eq!(missing.json()["detail"], "actor://counter/zz does not exist");
}

#[tokio::test(start_paused = true)]
async fn actor_health_serves_readiness_outside_authentication_and_503_while_the_store_is_down() {
    let store = FlakyStore::new();
    let system = node(store.clone()).await;
    let app = Router::new()
        .merge(actor_health(system.clone(), "/ready"))
        .nest("/probes", actor_health(system.clone(), "readyz"))
        .merge(authenticated(actor_api(system), BEARER));
    assert_eq!(get(&app, "/health").await.status, StatusCode::UNAUTHORIZED, "the API's own health stays protected");
    let ready = get(&app, "/ready").await;
    assert_eq!(ready.status, StatusCode::OK);
    assert_eq!(ready.json()["status"], "up");
    assert_eq!(get(&app, "/probes/readyz").await.status, StatusCode::OK);
    store.set_down(true);
    let down = get(&app, "/ready").await;
    assert_eq!(down.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(down.content_type(), PROBLEM_JSON);
    assert_eq!(down.json()["type"], pt::UNHEALTHY);
}

/// On the in-memory store: messages sent over HTTP, delayed ones included, are handled by
/// the dispatch loop, and a later node on the same store picks up what is left.
#[tokio::test(start_paused = true)]
async fn a_node_handles_messages_over_http_including_delayed_ones_and_a_later_node_continues() {
    let store = memory_store();
    let total = async |app: &Router| get_json(app, "/actors/counter/a").await["state"]["total"].clone();
    let first = node_with(store.clone(), |b| b.host(CounterActor).poll_interval(Duration::from_millis(100))).await;
    let app = actor_api(first.clone());
    assert_eq!(put(&app, "/actors/counter/a", r#"{"state": {}}"#).await.status, StatusCode::CREATED);
    for n in 1..=5 {
        let body = format!(r#"{{"type": "Add", "payload": {{"n": {n}}}}}"#);
        assert_eq!(post(&app, "/actors/counter/a/messages", &body).await.status, StatusCode::ACCEPTED);
    }
    eventually(async || total(&app).await == 15).await;

    let started = Instant::now();
    let delayed =
        send_id(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 100}, "delay": "PT0.3S"}"#)
            .await;
    assert_eq!(get_json(&app, &format!("/messages/{delayed}")).await["status"], "pending");
    eventually(async || get_json(&app, &format!("/messages/{delayed}")).await["status"] == "done").await;
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(total(&app).await, 115);
    assert_eq!(get_json(&app, "/actors/counter/a").await["version"], 6);

    // A message left for the next node.
    post(&app, "/actors/counter/a/messages", r#"{"type": "Add", "payload": {"n": 1000}, "delay": "PT1.5S"}"#).await;
    first.stop().await;

    let second = node_with(store, |b| b.host(CounterActor).poll_interval(Duration::from_millis(100))).await;
    let app = actor_api(second);
    assert_eq!(items(&get_json(&app, "/actors/counter/a/messages").await).len(), 1);
    eventually(async || total(&app).await == 1115).await;
    assert_eq!(items(&get_json(&app, "/types").await).len(), 1);
}
