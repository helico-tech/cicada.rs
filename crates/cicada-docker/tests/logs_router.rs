//! The logs router on a fake engine.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cicada_docker::{ContainerActor, ContainerSpec, Engine, URI_LABEL, container_logs};
use http_body_util::BodyExt;
use serde_json::Value;
use support::FakeDocker;
use tower::ServiceExt;

async fn fake_with_web() -> FakeDocker {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    let labels = BTreeMap::from([(URI_LABEL.to_owned(), "actor://containers/web".to_owned())]);
    fake.create("cicada-web", &ContainerSpec::new("nginx:1.27"), &labels).await.unwrap();
    let lines: Vec<String> = (1..=150).map(|i| format!("line {i}")).collect();
    fake.print("cicada-web", &lines.iter().map(String::as_str).collect::<Vec<_>>());
    fake
}

async fn get(fake: &FakeDocker, uri: &str) -> (StatusCode, String, String) {
    let router = container_logs(&ContainerActor::shared(Arc::new(fake.clone())));
    let response = router.oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let content_type = response.headers()[header::CONTENT_TYPE].to_str().unwrap().to_owned();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, content_type, String::from_utf8(body.to_vec()).unwrap())
}

fn problem(body: &str) -> Value {
    serde_json::from_str(body).unwrap()
}

#[tokio::test]
async fn streams_the_last_100_lines_by_default() {
    let fake = fake_with_web().await;
    let (status, content_type, body) = get(&fake, "/containers/web/logs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "text/plain; charset=utf-8");
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 100);
    assert_eq!(lines[0], "line 51");
    assert_eq!(lines[99], "line 150");
}

#[tokio::test]
async fn tail_selects_the_lines() {
    let fake = fake_with_web().await;
    assert_eq!(get(&fake, "/containers/web/logs?tail=2").await.2, "line 149\nline 150\n");
    assert_eq!(get(&fake, "/containers/web/logs?tail=all").await.2.lines().count(), 150);
}

#[tokio::test]
async fn unknown_and_foreign_containers_are_not_found() {
    let fake = fake_with_web().await;
    let (status, content_type, body) = get(&fake, "/containers/db/logs").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(content_type, "application/problem+json");
    assert_eq!(problem(&body)["type"], "urn:cicada:problem:not-found");
    assert_eq!(problem(&body)["instance"], "/containers/db/logs");

    fake.put_foreign("cicada-other");
    assert_eq!(get(&fake, "/containers/other/logs").await.0, StatusCode::NOT_FOUND);
    assert_eq!(get(&fake, "/containers/bad%20name/logs").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_bad_query_is_a_bad_request() {
    let fake = fake_with_web().await;
    for query in ["tail=-1", "tail=lots", "follow=maybe", "since=yesterday"] {
        let (status, _, body) = get(&fake, &format!("/containers/web/logs?{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(problem(&body)["type"], "urn:cicada:problem:bad-request");
    }
}

#[tokio::test]
async fn docker_down_is_a_bad_gateway() {
    let fake = fake_with_web().await;
    fake.set_down(true);
    let (status, _, body) = get(&fake, "/containers/web/logs").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(problem(&body)["type"], "urn:cicada:problem:docker-unavailable");
}
