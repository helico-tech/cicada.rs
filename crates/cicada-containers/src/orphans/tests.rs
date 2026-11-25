//! The watch on virtual time, with the fake runtime and `TestActors`, which hosts the job so that a test can create
//! its actors; a stand-in for the system fails actor reads as scripted. Rounds run at 0, 5, 10 minutes and so on; tests
//! look between them.

use std::sync::Mutex;

use cicada::prelude::JsonSchema;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use super::*;
use crate::{Call, ContainerActor, ContainerSettings, ContainerSpec, ContainerState, ErrorKind, FakeRuntime, Method};

const MINUTE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Run {}

struct Tests;

impl ContainerJob for Tests {
    const TYPE: &'static str = "ci.tests";
    type Request = Run;

    fn spec(&self, name: &str, _: &Run) -> Result<ContainerSpec, String> {
        Ok(ContainerSpec::new(name, "alpine:3"))
    }
}

/// A node that hosts the job with the prefix `ci-` on `fake`.
async fn actors(fake: &FakeRuntime) -> TestActors {
    let runtime: Arc<dyn ContainerRuntime> = Arc::new(fake.clone());
    let settings = ContainerSettings::new("ci-").unwrap();
    TestActors::builder()
        .with(move |node| node.host(ContainerActor::new(Tests, runtime, settings)))
        .build()
        .await
        .unwrap()
}

/// Creates and starts the container `name` with the label of `prefix`.
async fn labelled(fake: &FakeRuntime, name: &str, prefix: &str) {
    let mut spec = ContainerSpec::new(name, "alpine:3");
    spec.labels.insert(PREFIX_LABEL.into(), prefix.into());
    fake.create(&spec).await.unwrap();
    fake.start(name).await.unwrap();
}

/// Creates the actor keyed `key`, without running a turn.
async fn create_actor(test: &TestActors, key: &str) {
    test.create(&Container::<Tests>::named(key), &ContainerState::new(Run {})).await;
}

/// A system that fails the next `fails` actor reads, then reads through the real one.
struct Failing {
    system: ActorSystem,
    fails: Mutex<usize>,
}

#[async_trait]
impl Actors for Failing {
    async fn exists(&self, address: &ActorAddress) -> Result<bool, OpError> {
        {
            let mut fails = self.fails.lock().unwrap();
            if *fails > 0 {
                *fails -= 1;
                return Err(OpError::Invalid("scripted".into()));
            }
        }
        self.system.exists(address).await
    }
}

/// A running watch of the prefix `ci-`, with the names it reported.
struct Running {
    reported: Arc<Mutex<Vec<String>>>,
    stop: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

impl Running {
    fn reported(&self) -> Vec<String> {
        self.reported.lock().unwrap().clone()
    }

    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.handle.await.unwrap();
    }
}

fn watch(fake: &FakeRuntime) -> (OrphanWatch<Tests>, Arc<Mutex<Vec<String>>>) {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let seen = reported.clone();
    let watch = OrphanWatch::<Tests>::new(Arc::new(fake.clone()), "ci-", OrphanSettings::new(), move |name: &str| {
        seen.lock().unwrap().push(name.to_owned());
    })
    .unwrap();
    (watch, reported)
}

fn start(fake: &FakeRuntime, test: &TestActors) -> Running {
    let (watch, reported) = watch(fake);
    let (stop, stopped) = oneshot::channel();
    let handle = tokio::spawn(watch.run(test.system().clone(), async move {
        let _ = stopped.await;
    }));
    Running { reported, stop, handle }
}

fn start_failing(fake: &FakeRuntime, test: &TestActors, fails: usize) -> Running {
    let (watch, reported) = watch(fake);
    let (stop, stopped) = oneshot::channel();
    let actors = Failing { system: test.system().clone(), fails: Mutex::new(fails) };
    let handle = tokio::spawn(async move {
        watch
            .run_on(&actors, async move {
                let _ = stopped.await;
            })
            .await;
    });
    Running { reported, stop, handle }
}

/// Scenario: A container left without its actor. One with its actor is not reported.
#[tokio::test(start_paused = true)]
async fn a_container_without_its_actor_is_reported_once_after_the_second_round() {
    let fake = FakeRuntime::new();
    let test = actors(&fake).await;
    labelled(&fake, "ci-left", "ci-").await;
    labelled(&fake, "ci-owned", "ci-").await;
    create_actor(&test, "owned").await;
    let running = start(&fake, &test);

    sleep(MINUTE).await;
    assert!(running.reported().is_empty(), "not after the first round");
    sleep(5 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-left"]);
    sleep(20 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-left"], "once");
    assert!(fake.inspect("ci-left").await.is_ok(), "not removed");
    assert!(!fake.calls().iter().any(|call| matches!(call, Call::Stop { .. } | Call::Remove(_) | Call::Sweep(_))));
    running.stop().await;
}

/// An actor missing in one round only is no orphan.
#[tokio::test(start_paused = true)]
async fn an_actor_missing_in_one_round_only_is_not_reported() {
    let fake = FakeRuntime::new();
    let test = actors(&fake).await;
    labelled(&fake, "ci-new", "ci-").await;
    let running = start(&fake, &test);

    sleep(MINUTE).await;
    create_actor(&test, "new").await;
    sleep(20 * MINUTE).await;
    assert!(running.reported().is_empty());
    running.stop().await;
}

/// Scenario: An actor that appears. Gone again, it is reported again after two rounds.
#[tokio::test(start_paused = true)]
async fn an_orphan_whose_actor_appears_is_dropped_and_not_reported_again() {
    let fake = FakeRuntime::new();
    let test = actors(&fake).await;
    labelled(&fake, "ci-a", "ci-").await;
    let running = start(&fake, &test);

    sleep(6 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a"]);
    create_actor(&test, "a").await;
    sleep(20 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a"], "not while its actor exists");

    test.system().delete_actor(&Container::<Tests>::named("a").address().unwrap()).await.unwrap();
    sleep(5 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a"], "one round without it");
    sleep(5 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a", "ci-a"]);
    running.stop().await;
}

/// Scenario: A removed orphan. A new container of that name starts unreported.
#[tokio::test(start_paused = true)]
async fn a_removed_orphan_leaves_the_set() {
    let fake = FakeRuntime::new();
    let test = actors(&fake).await;
    labelled(&fake, "ci-a", "ci-").await;
    let running = start(&fake, &test);

    sleep(6 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a"]);
    fake.remove("ci-a").await.unwrap();
    sleep(5 * MINUTE).await; // a round without it
    labelled(&fake, "ci-a", "ci-").await;
    sleep(5 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a"], "the new one's first round");
    sleep(5 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a", "ci-a"]);
    running.stop().await;
}

/// Scenario: A failed list. The failed round does not count: still one round without the actor after it.
#[tokio::test(start_paused = true)]
async fn a_failed_list_is_tried_next_round_and_does_not_count() {
    let fake = FakeRuntime::new();
    let test = actors(&fake).await;
    labelled(&fake, "ci-a", "ci-").await;
    let running = start(&fake, &test);

    sleep(MINUTE).await; // round 0 counted
    fake.fail_next(Method::List, ErrorKind::DaemonUnavailable);
    sleep(5 * MINUTE).await; // round 5 failed
    assert!(running.reported().is_empty());
    sleep(5 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a"], "at the second counted round");
    running.stop().await;
}

/// A failed actor read neither counts nor resets the count.
#[tokio::test(start_paused = true)]
async fn a_failed_actor_read_is_tried_next_round_and_does_not_count() {
    let fake = FakeRuntime::new();
    let test = actors(&fake).await;
    labelled(&fake, "ci-a", "ci-").await;
    let running = start_failing(&fake, &test, 2);

    sleep(6 * MINUTE).await; // rounds 0 and 5 failed
    assert!(running.reported().is_empty());
    sleep(5 * MINUTE).await; // round 10 counted
    assert!(running.reported().is_empty());
    sleep(5 * MINUTE).await;
    assert_eq!(running.reported(), ["ci-a"]);
    running.stop().await;
}

/// A labelled container without the prefix, or without a valid key, is skipped; another job's label is not listed.
#[tokio::test(start_paused = true)]
async fn a_name_without_the_prefix_is_skipped() {
    let fake = FakeRuntime::new();
    let test = actors(&fake).await;
    labelled(&fake, "other-1", "ci-").await;
    labelled(&fake, "ci-", "ci-").await; // the empty key
    labelled(&fake, "ci-x", "other-").await;
    let running = start(&fake, &test);

    sleep(30 * MINUTE).await;
    assert!(running.reported().is_empty());
    running.stop().await;
}

#[test]
fn a_zero_interval_is_refused() {
    let mut settings = OrphanSettings::new();
    settings.interval = Duration::ZERO;
    let refused = OrphanWatch::<Tests>::new(Arc::new(FakeRuntime::new()), "ci-", settings, |_: &str| {});
    assert_eq!(refused.unwrap_err().to_string(), "interval is zero");
}

/// `stop` ends `run`.
#[tokio::test(start_paused = true)]
async fn stop_ends_run() {
    let fake = FakeRuntime::new();
    let test = actors(&fake).await;
    let running = start(&fake, &test);
    sleep(11 * MINUTE).await;
    running.stop().await;

    let (watch, _) = watch(&fake);
    let lists = fake.calls().iter().filter(|call| matches!(call, Call::List(_))).count();
    assert_eq!(lists, 3, "rounds at 0, 5 and 10 minutes");
    watch.run(test.system().clone(), sleep(MINUTE)).await;
}
