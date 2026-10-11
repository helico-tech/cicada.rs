//! The container handle on a fake engine, on virtual time.

mod support;

use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada_docker::{
    ContainerActor, ContainerHandle, ContainerMsg, ContainerSpec, ContainerState, ContainerStatus, EngineError,
    PullPolicy, SPEC_HASH_LABEL, URI_LABEL,
};
use cicada_test::TestActors;
use support::FakeDocker;

const NAME: &str = "cicada-web";

fn web() -> ContainerHandle {
    ContainerHandle::new("web")
}

fn spec() -> ContainerSpec {
    ContainerSpec::new("nginx:1.27").port(80, 8080)
}

async fn harness(fake: &FakeDocker) -> TestActors {
    let actor = ContainerActor::shared(Arc::new(fake.clone()));
    let test = TestActors::builder().with(move |node| node.host(actor)).build().await.unwrap();
    test.create(&web(), &ContainerState::default()).await;
    test
}

async fn state(test: &TestActors) -> ContainerState {
    test.state(&web()).await
}

async fn apply(test: &TestActors, spec: ContainerSpec) -> cicada::store::Message {
    let sent = test.send(&web(), ContainerMsg::Apply { spec }).await;
    test.run_until_idle().await;
    sent
}

fn retry(attempts: u32) -> SendOptions {
    SendOptions::new().retry(RetryPolicy::new(attempts, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap())
}

#[tokio::test(start_paused = true)]
async fn apply_pulls_a_missing_image_creates_starts_and_records() {
    let fake = FakeDocker::default();
    let test = harness(&fake).await;
    apply(&test, spec()).await;

    assert_eq!(
        fake.calls(),
        [
            "inspect cicada-web",
            "image_present nginx:1.27",
            "pull nginx:1.27",
            "create cicada-web",
            "start cicada-web",
            "inspect cicada-web",
        ]
    );
    let container = fake.container(NAME).unwrap();
    assert_eq!(container.status, ContainerStatus::Running);
    assert_eq!(container.labels[URI_LABEL], "actor://containers/web");
    assert_eq!(container.labels[SPEC_HASH_LABEL], spec().hash());

    let s = state(&test).await;
    assert_eq!(s.spec, Some(spec()));
    let observed = s.observed.unwrap();
    assert_eq!(observed.status, ContainerStatus::Running);
    assert_eq!(observed.container_id.as_deref(), Some(container.id.as_str()));
    assert_eq!(test.pending(&web()).await.len(), 1, "one Check is pending");
}

#[tokio::test(start_paused = true)]
async fn applying_the_same_spec_again_makes_no_second_container_and_keeps_one_check() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    let test = harness(&fake).await;
    apply(&test, spec()).await;
    apply(&test, spec()).await;

    assert_eq!(fake.count("create cicada-web"), 1);
    assert_eq!(fake.count("pull nginx:1.27"), 0, "the image was present");
    let pending = test.pending(&web()).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(Some(&pending[0].id), state(&test).await.next_check.as_ref());
}

#[tokio::test(start_paused = true)]
async fn a_changed_spec_replaces_the_container() {
    let fake = FakeDocker::with_images(&["nginx:1.27", "nginx:1.28"]);
    let test = harness(&fake).await;
    apply(&test, spec()).await;
    let first = fake.container(NAME).unwrap().id;

    let newer = ContainerSpec::new("nginx:1.28").port(80, 8080);
    apply(&test, newer.clone()).await;

    let container = fake.container(NAME).unwrap();
    assert_ne!(container.id, first);
    assert_eq!(container.spec, newer);
    assert_eq!(container.labels[SPEC_HASH_LABEL], newer.hash());
    assert_eq!(container.status, ContainerStatus::Running);
    assert_eq!(fake.count("remove cicada-web"), 1);
}

#[tokio::test(start_paused = true)]
async fn a_retried_apply_finds_the_container_of_the_failed_attempt() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    fake.fail_once("start", EngineError::Other("daemon hiccup".into()));
    let test = harness(&fake).await;
    let sent = test.send_with(&web(), ContainerMsg::Apply { spec: spec() }, retry(3)).await;
    test.run_until_idle().await;
    test.advance_time_by(Duration::from_secs(2)).await;

    assert_eq!(test.outcome(&sent).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(fake.count("create cicada-web"), 1, "the second attempt reused the container");
    assert_eq!(fake.container(NAME).unwrap().status, ContainerStatus::Running);
}

#[tokio::test(start_paused = true)]
async fn a_container_of_someone_else_is_never_touched() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    fake.put_foreign(NAME);
    let test = harness(&fake).await;
    let sent = test.send_with(&web(), ContainerMsg::Apply { spec: spec() }, retry(3)).await;
    test.run_until_idle().await;

    let outcome = test.outcome(&sent).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed, "permanent: not retried");
    assert!(outcome.error.unwrap().contains("does not belong to actor://containers/web"));
    assert_eq!(fake.container(NAME).unwrap().id, "foreign");
    assert!(!fake.calls().iter().any(|c| c.starts_with("remove") || c.starts_with("stop")));
}

#[tokio::test(start_paused = true)]
async fn check_records_an_exit_with_code_and_last_lines() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    let test = harness(&fake).await;
    apply(&test, spec()).await;
    fake.print(NAME, &["starting"]);
    fake.exit(NAME, 3, &["config error", "giving up"]);

    test.advance_time_by(ContainerActor::DEFAULT_CHECK_EVERY).await;
    let observed = state(&test).await.observed.unwrap();
    assert_eq!(observed.status, ContainerStatus::Exited);
    assert_eq!(observed.exit_code, Some(3));
    assert_eq!(observed.last_lines, ["starting", "config error", "giving up"]);
    assert_eq!(fake.count("logs cicada-web"), 1);

    test.advance_time_by(ContainerActor::DEFAULT_CHECK_EVERY).await;
    assert_eq!(fake.count("logs cicada-web"), 1, "the same run's lines are not read again");
    assert_eq!(fake.container(NAME).unwrap().status, ContainerStatus::Exited, "no restart by the handle");
    assert_eq!(test.pending(&web()).await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn polling_survives_docker_being_down() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    let test = harness(&fake).await;
    apply(&test, spec()).await;

    fake.set_down(true);
    test.advance_time_by(ContainerActor::DEFAULT_CHECK_EVERY).await;
    let observed = state(&test).await.observed.unwrap();
    assert_eq!(observed.status, ContainerStatus::Running, "the last good look is kept");
    assert_eq!(observed.check_error.as_deref(), Some("Docker daemon unreachable"));
    assert_eq!(test.pending(&web()).await.len(), 1, "the next check is armed");
    assert!(test.failures().await.is_empty());

    fake.set_down(false);
    test.advance_time_by(ContainerActor::DEFAULT_CHECK_EVERY).await;
    assert_eq!(state(&test).await.observed.unwrap().check_error, None);
}

#[tokio::test(start_paused = true)]
async fn stop_and_start() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    let test = harness(&fake).await;
    apply(&test, spec()).await;

    test.send(&web(), ContainerMsg::Stop).await;
    test.run_until_idle().await;
    assert_eq!(state(&test).await.observed.unwrap().status, ContainerStatus::Exited);

    test.send(&web(), ContainerMsg::Start).await;
    test.run_until_idle().await;
    assert_eq!(state(&test).await.observed.unwrap().status, ContainerStatus::Running);
    assert_eq!(fake.count("create cicada-web"), 1);
}

#[tokio::test(start_paused = true)]
async fn start_recreates_a_container_removed_outside_cicada() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    let test = harness(&fake).await;
    apply(&test, spec()).await;
    fake.state().containers.clear();

    test.advance_time_by(ContainerActor::DEFAULT_CHECK_EVERY).await;
    assert_eq!(state(&test).await.observed.unwrap().status, ContainerStatus::Missing);
    assert_eq!(fake.count("create cicada-web"), 1, "a check does not recreate");

    test.send(&web(), ContainerMsg::Start).await;
    test.run_until_idle().await;
    assert_eq!(fake.count("create cicada-web"), 2);
    assert_eq!(state(&test).await.observed.unwrap().status, ContainerStatus::Running);
}

#[tokio::test(start_paused = true)]
async fn start_without_a_spec_fails_permanently() {
    let fake = FakeDocker::default();
    let test = harness(&fake).await;
    let sent = test.send_with(&web(), ContainerMsg::Start, retry(3)).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&sent).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert!(outcome.error.unwrap().contains("send Apply first"));
}

#[tokio::test(start_paused = true)]
async fn remove_removes_the_container_and_the_handle() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    let test = harness(&fake).await;
    apply(&test, spec()).await;

    test.send(&web(), ContainerMsg::Remove).await;
    test.run_until_idle().await;
    assert!(fake.container(NAME).is_none());
    assert!(!test.exists(&web()).await);
    assert!(test.pending(&web()).await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn pull_never_with_a_missing_image_fails() {
    let fake = FakeDocker::default();
    let test = harness(&fake).await;
    let sent = apply(&test, spec().pull(PullPolicy::Never)).await;
    let outcome = test.outcome(&sent).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert!(outcome.error.unwrap().contains("No such image"));
    assert_eq!(fake.count("pull nginx:1.27"), 0);
}

#[tokio::test(start_paused = true)]
async fn reserved_labels_and_bad_names_fail_permanently() {
    let fake = FakeDocker::with_images(&["nginx:1.27"]);
    let test = harness(&fake).await;
    let mut labelled = spec();
    labelled.labels.insert("cicada.uri".into(), "actor://containers/other".into());
    let sent = test.send_with(&web(), ContainerMsg::Apply { spec: labelled }, retry(3)).await;
    test.run_until_idle().await;
    assert!(test.outcome(&sent).await.unwrap().error.unwrap().contains("reserved"));

    let bad = ContainerHandle::new("no spaces");
    test.create(&bad, &ContainerState::default()).await;
    let sent = test.send_with(&bad, ContainerMsg::Apply { spec: spec() }, retry(3)).await;
    test.run_until_idle().await;
    let outcome = test.outcome(&sent).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert!(outcome.error.unwrap().contains("Invalid container handle name"));
    assert!(fake.calls().is_empty());
}
