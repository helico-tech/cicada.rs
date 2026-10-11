//! The container handle and the logs router against a real Docker daemon. Runs only when `CICADA_DOCKER=1`; pulls
//! `alpine:3.20` when absent. Containers get a prefix of their own and are removed even when a test fails.

use std::process::Command;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cicada::store::OutcomeStatus;
use cicada_docker::{
    ContainerActor, ContainerHandle, ContainerMsg, ContainerSpec, ContainerState, ContainerStatus, DockerEngine,
    Observed, container_logs,
};
use cicada_test::TestActors;
use http_body_util::BodyExt;
use tower::ServiceExt;

const IMAGE: &str = "alpine:3.20";

fn enabled() -> bool {
    let on = std::env::var("CICADA_DOCKER").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set CICADA_DOCKER=1 to run against Docker");
    }
    on
}

/// Removes the test's container when dropped, also after a failed assertion.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

struct Fixture {
    test: TestActors,
    containers: ContainerActor,
    handle: ContainerHandle,
    _cleanup: Cleanup,
}

async fn fixture(name: &str) -> Fixture {
    let prefix = format!("cicada-test-{}-", std::process::id());
    let containers = ContainerActor::new(DockerEngine::connect().unwrap()).prefix(prefix.clone());
    let hosted = containers.clone();
    let test = TestActors::builder().with(move |node| node.host(hosted)).build().await.unwrap();
    let handle = ContainerHandle::new(name);
    test.create(&handle, &ContainerState::default()).await;
    Fixture { test, containers, handle, _cleanup: Cleanup(format!("{prefix}{name}")) }
}

fn sh(script: &str) -> ContainerSpec {
    let mut spec = ContainerSpec::new(IMAGE).cmd(["sh", "-c", script]);
    spec.stop_timeout_seconds = Some(1);
    spec
}

impl Fixture {
    async fn command(&self, msg: ContainerMsg) {
        let sent = self.test.send(&self.handle, msg).await;
        self.test.run_until_idle().await;
        let outcome = self.test.outcome(&sent).await;
        if let Some(outcome) = outcome {
            assert_eq!(outcome.status, OutcomeStatus::Done, "{:?}", outcome.error);
        }
    }

    async fn observed(&self) -> Observed {
        self.test.state::<_, ContainerState>(&self.handle).await.observed.expect("observed")
    }

    /// Sends `Check` until `done` holds, for at most 20 s.
    async fn check_until(&self, done: impl Fn(&Observed) -> bool) -> Observed {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.command(ContainerMsg::Check).await;
            let observed = self.observed().await;
            if done(&observed) {
                return observed;
            }
            assert!(Instant::now() < deadline, "gave up waiting; last {observed:?}");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn logs(&self, query: &str) -> (StatusCode, String) {
        let uri = format!("/containers/{}/logs?{query}", self.handle.name);
        let response =
            container_logs(&self.containers).oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }
}

#[tokio::test]
async fn a_container_through_its_whole_life() {
    if !enabled() {
        return;
    }
    let f = fixture("life").await;

    f.command(ContainerMsg::Apply { spec: sh("echo hello; echo oops >&2; sleep 300") }).await;
    let observed = f.observed().await;
    assert_eq!(observed.status, ContainerStatus::Running);
    assert_eq!(observed.exit_code, None, "no exit code while it runs");
    let first_id = observed.container_id.clone().unwrap();

    let (status, body) = f.logs("tail=all").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("hello") && body.contains("oops"), "{body}");

    f.command(ContainerMsg::Start).await; // already running: harmless
    f.command(ContainerMsg::Stop).await;
    f.command(ContainerMsg::Stop).await; // already stopped: harmless
    let observed = f.observed().await;
    assert_eq!(observed.status, ContainerStatus::Exited);
    assert!(observed.exit_code.is_some());

    f.command(ContainerMsg::Apply { spec: sh("echo bye; exit 4") }).await;
    let observed = f.check_until(|o| o.status == ContainerStatus::Exited).await;
    assert_ne!(observed.container_id.as_deref(), Some(first_id.as_str()), "replaced");
    assert_eq!(observed.exit_code, Some(4));
    assert_eq!(observed.last_lines, ["bye"]);

    f.command(ContainerMsg::Remove).await;
    assert!(!f.test.exists(&f.handle).await);
    let name = f.containers.container_name(&f.handle).unwrap();
    assert!(f.containers.engine().inspect(&name).await.unwrap().is_none());
    assert_eq!(f.logs("").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn follow_streams_until_the_container_exits() {
    if !enabled() {
        return;
    }
    let f = fixture("follow").await;
    f.command(ContainerMsg::Apply { spec: sh("for i in 1 2 3; do echo tick $i; sleep 1; done") }).await;
    let (status, body) = tokio::time::timeout(Duration::from_secs(20), f.logs("follow=true&tail=all")).await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "tick 1\ntick 2\ntick 3\n");
}
