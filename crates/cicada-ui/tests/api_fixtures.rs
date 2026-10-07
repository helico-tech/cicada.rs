//! Port of ApiFixturesTest (actors-ui): records real responses of every endpoint of the Rust API (successes and
//! problems) as JSON files with a manifest naming each file's schema, then runs the console's vitest check
//! (`web/test/fixtures.test.ts`), which parses every file with the validators generated from the wire classes. A response
//! the console cannot read fails the test. The vitest step is skipped when the console's node_modules are missing.

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cicada::prelude::*;
use cicada::store::SystemClock;
use cicada_test::InMemoryStore;
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower::ServiceExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Fixture {
    id: String,
}

impl ActorRef for Fixture {
    const TEMPLATE: &'static str = "actor://fixture/{id}";
    type Message = FixtureMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum FixtureMsg {
    Add {
        n: i32,
        #[serde(default)]
        note: Option<String>,
    },
    Fail {
        reason: String,
    },
    /// Sends `Add(n)` to each fixture in `to`.
    Forward {
        to: Vec<String>,
        n: i32,
    },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct FixtureState {
    #[serde(default)]
    total: i32,
    #[serde(default)]
    tags: Vec<String>,
}

struct FixtureActor;

impl Actor for FixtureActor {
    type Ref = Fixture;
    type State = FixtureState;
    type Message = FixtureMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: FixtureMsg) -> HandlerResult {
        match message {
            FixtureMsg::Add { n, .. } => turn.state_mut().total += n,
            FixtureMsg::Fail { reason } => return Err(HandlerError::msg(reason)),
            FixtureMsg::Forward { to, n } => {
                for id in to {
                    turn.send(&Fixture { id }, FixtureMsg::Add { n, note: None })?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Beacon {
    name: String,
}

impl ActorRef for Beacon {
    const TEMPLATE: &'static str = "actor://beacon/{name}";
    type Message = BeaconMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum BeaconMsg {
    Blink,
}

struct BeaconActor;

impl Actor for BeaconActor {
    type Ref = Beacon;
    type State = Stateless;
    type Message = BeaconMsg;
    const UNHANDLED: Unhandled = Unhandled::Drop;

    async fn handle(&self, _: &mut Turn<Self>, _: BeaconMsg) -> HandlerResult {
        Ok(())
    }
}

struct Recorder {
    app: Router,
    dir: std::path::PathBuf,
    manifest: Vec<(String, String)>,
}

impl Recorder {
    async fn call(&self, method: Method, path: &str, body: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn record(
        &mut self,
        name: &str,
        schema: &str,
        method: Method,
        path: &str,
        body: &str,
        status: StatusCode,
    ) -> Value {
        let (actual, text) = self.call(method, path, body).await;
        assert_eq!(actual, status, "{name}: {text}");
        std::fs::write(self.dir.join(format!("{name}.json")), &text).unwrap();
        self.manifest.push((format!("{name}.json"), schema.to_owned()));
        serde_json::from_str(&text).unwrap()
    }

    async fn status(&self, id: &str) -> Value {
        serde_json::from_str(&self.call(Method::GET, &format!("/messages/{id}"), "").await.1).unwrap()
    }

    async fn eventually(&self, what: &str, mut condition: impl AsyncFnMut(&Self) -> bool) {
        let start = Instant::now();
        while !condition(self).await {
            assert!(start.elapsed() < Duration::from_secs(10), "{what}: not met within 10 s");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn record_a_response_of_every_endpoint_and_parse_them_with_the_console_types() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("api-fixtures");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Arc::new(InMemoryStore::with_clock(Arc::new(SystemClock)));
    let system = ActorSystem::builder(store)
        .host(FixtureActor)
        .host(BeaconActor)
        .poll_interval(Duration::from_millis(50))
        .build();
    system.start().await.unwrap();
    let mut r = Recorder { app: cicada_axum::actor_api(system.clone()), dir: dir.clone(), manifest: Vec::new() };
    use Method as M;
    use StatusCode as S;

    r.record(
        "create-created",
        "CreateActorResponse",
        M::PUT,
        "/actors/fixture/a",
        r#"{"state": {"tags": ["x"]}}"#,
        S::CREATED,
    )
    .await;
    r.record("create-existing", "CreateActorResponse", M::PUT, "/actors/fixture/a", r#"{"state": {}}"#, S::OK).await;
    r.call(M::PUT, "/actors/fixture/b", r#"{"state": {}}"#).await;
    r.call(M::PUT, "/actors/beacon/one", "").await;

    let forward_body = r#"{"id": "m-forward", "type": "Forward", "payload": {"to": ["b"], "n": 2}}"#;
    let sent = r
        .record("send-accepted", "SendResponse", M::POST, "/actors/fixture/a/messages", forward_body, S::ACCEPTED)
        .await;
    r.record("send-duplicate", "SendResponse", M::POST, "/actors/fixture/a/messages", forward_body, S::OK).await;
    let forward = sent["id"].as_str().unwrap().to_owned();
    r.eventually("forward done", async |r| r.status(&forward).await["status"] == "done").await;
    r.record("message-done", "MessageStatusView", M::GET, &format!("/messages/{forward}"), "", S::OK).await;

    // A failed message with a retry policy that ran out, and a delayed one with policies.
    r.call(
        M::POST,
        "/actors/fixture/b/messages",
        r#"{"id": "m-fail", "type": "Fail", "payload": {"reason": "boom"},
            "retry": {"maxAttempts": 2, "backoff": {"kind": "fixed", "delay": "PT0.1S"}},
            "retention": {"failed": {"kind": "for", "age": "P30D"}}}"#,
    )
    .await;
    r.eventually("m-fail failed", async |r| r.status("m-fail").await["status"] == "failed").await;
    r.record("message-failed", "MessageStatusView", M::GET, "/messages/m-fail", "", S::OK).await;
    r.call(
        M::POST,
        "/actors/fixture/a/messages",
        r#"{"id": "m-later", "type": "Add", "payload": {"n": 1}, "delay": "PT1H", "correlationId": "c-1",
            "headers": {"trace": "t"}, "retry": {"maxAttempts": 3, "backoff": {"kind": "exponential", "initial": "PT1S", "max": "PT1M"}},
            "retention": {"done": {"kind": "last", "count": 10}}}"#,
    )
    .await;
    r.record("message-pending", "MessageStatusView", M::GET, "/messages/m-later", "", S::OK).await;

    // A removed cause in a chain.
    r.call(
        M::POST,
        "/actors/fixture/a/messages",
        r#"{"id": "m-removed", "type": "Forward", "payload": {"to": ["b"], "n": 1}, "retention": {"done": {"kind": "remove"}}}"#,
    )
    .await;
    r.eventually("m-removed gone", async |r| {
        r.call(Method::GET, "/messages/m-removed", "").await.0 == StatusCode::NOT_FOUND
    })
    .await;
    r.record("chain", "MessageChainView", M::GET, &format!("/messages/{forward}/chain"), "", S::OK).await;
    let removed = r.record("chain-removed", "MessageChainView", M::GET, "/messages/m-removed/chain", "", S::OK).await;
    let focus = removed["nodes"].as_array().unwrap().iter().find(|n| n["id"] == removed["focus"]).unwrap();
    assert_eq!(focus["removed"], true);

    r.record("actor", "ActorView", M::GET, "/actors/fixture/a", "", S::OK).await;
    r.record("actor-stateless", "ActorView", M::GET, "/actors/beacon/one", "", S::OK).await;
    r.record("actors", "ActorPage", M::GET, "/actors?prefix=actor://fixture/&limit=1", "", S::OK).await;
    r.record("mailbox", "MessagePage", M::GET, "/actors/fixture/a/messages", "", S::OK).await;
    r.record("pending", "MessagePage", M::GET, "/messages?prefix=actor://", "", S::OK).await;
    r.record("failures", "OutcomePage", M::GET, "/failures", "", S::OK).await;
    r.record("types", "TypeListView", M::GET, "/types", "", S::OK).await;
    r.record("type-match", "TypeMatchView", M::GET, "/types?uri=actor://fixture/a", "", S::OK).await;
    r.record("health", "HealthView", M::GET, "/health", "", S::OK).await;

    r.record("problem-no-such-instance", "Problem", M::GET, "/actors/fixture/nope", "", S::NOT_FOUND).await;
    r.record("problem-no-such-message", "Problem", M::GET, "/messages/nope", "", S::NOT_FOUND).await;
    let bad_payload = r#"{"type": "Add", "payload": {"n": "x"}}"#;
    r.record(
        "problem-invalid-payload",
        "Problem",
        M::POST,
        "/actors/fixture/a/messages",
        bad_payload,
        S::UNPROCESSABLE_ENTITY,
    )
    .await;
    let bad_retention = r#"{"type": "Add", "retention": {"done": {"kind": "never"}}}"#;
    r.record("problem-bad-request", "Problem", M::POST, "/actors/fixture/a/messages", bad_retention, S::BAD_REQUEST)
        .await;
    r.record("problem-finished", "Problem", M::DELETE, &format!("/messages/{forward}"), "", S::CONFLICT).await;

    let manifest: serde_json::Map<String, Value> =
        r.manifest.iter().map(|(f, s)| (f.clone(), Value::String(s.clone()))).collect();
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    system.stop().await;

    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
    if !web.join("node_modules").exists() {
        eprintln!("skipping the vitest check: {} has no node_modules", web.display());
        return;
    }
    let npx = if cfg!(windows) { "npx.cmd" } else { "npx" };
    let output = Command::new(npx)
        .args(["vitest", "run", "test/fixtures.test.ts"])
        .env("CICADA_API_FIXTURES", &dir)
        .current_dir(&web)
        .output()
        .expect("npx runs");
    assert!(
        output.status.success(),
        "the console cannot read the API's responses:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
