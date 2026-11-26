//! The duplicate check of the placement member (ADR 0071), on virtual time with `TestActors`, in a test binary of its
//! own as it reads what the member logs: two started nodes under one node id and instance (a copied identity folder)
//! with different host names on one store, where the member that finds the other host in the node row skips its offers,
//! logs one error and reports `Duplicate`, and the pool alerts once; a redeploy under a new host name, whose old node
//! removed its row, trips nothing.

use std::fs;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::NodeConfig;
use cicada::store::NodeId;
use cicada_actors::{
    Capacity, MemberSettings, NodeIdentity, Placement, PlacementActor, PlacementMember, PlacementMsg, PlacementNotice,
    PlacementNoticeKind, PlacementState,
};
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn pool() -> Placement {
    Placement::named("runners")
}

/// `ops/pager`: keeps the notices of the pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ops;

impl ActorRef for Ops {
    const TYPE: &'static str = "ops";
    type Message = OpsMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum OpsMsg {
    /// A notice of a placement pool.
    Placement(PlacementNotice),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Notices {
    notices: Vec<PlacementNotice>,
}

struct OpsActor;

impl Actor for OpsActor {
    type Ref = Ops;
    type State = Notices;

    async fn handle(&self, turn: &mut Turn<Self>, message: OpsMsg) -> HandlerResult {
        let OpsMsg::Placement(notice) = message;
        turn.state_mut().notices.push(notice);
        Ok(())
    }
}

/// What the subscriber wrote.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Log {
    /// The error lines that hold `text`.
    fn errors(&self, text: &str) -> usize {
        let log = String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned();
        log.lines().filter(|line| line.contains("ERROR") && line.contains(text)).count()
    }
}

/// The test's node, which runs the pool `runners` (silence 60 s) with its notices going to `ops/pager`.
async fn pool_node() -> TestActors {
    let test = TestActors::builder()
        .start_at("2026-01-01T00:00:00Z".parse().unwrap())
        .with(|node| node.host(PlacementActor).host(OpsActor))
        .build()
        .await
        .unwrap();
    test.create(&Ops, &Notices::default()).await;
    test.create(&pool(), &PlacementState::new(secs(60), 10, 10, 10).unwrap()).await;
    test.send(&pool(), PlacementMsg::alerts(&Ops, "Placement").unwrap()).await;
    test.run_until_idle().await;
    test
}

/// A started send-only node on the test's store under the node id `shared` that writes `host` to its row.
async fn machine(test: &TestActors, host: &str) -> ActorSystem {
    let config =
        NodeConfig { node_id: NodeId::new("shared").unwrap(), hostname: Some(host.into()), ..NodeConfig::default() };
    let system = ActorSystem::builder(test.store().clone()).config(config).build();
    system.start().await.unwrap();
    system
}

/// Runs a member of `system` under `identity` with a probe of two places, until the returned sender is used.
fn member(system: &ActorSystem, identity: NodeIdentity) -> (oneshot::Sender<()>, JoinHandle<()>) {
    let member =
        PlacementMember::new(pool(), identity, MemberSettings::new(), || async { Some(Capacity::new(["arm64"], 2)) });
    let (stop, stopped) = oneshot::channel::<()>();
    let run = tokio::spawn(member.run(system.clone(), async move {
        let _ = stopped.await;
    }));
    (stop, run)
}

/// Lets the started nodes and the members take their turns at the current time.
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

/// The messages of `message_type` the pool handled.
async fn handled(test: &TestActors, message_type: &str) -> usize {
    let page = test.system().outcomes(None, "cicada.placement/", None, None, 500).await.unwrap();
    page.items.iter().filter(|o| o.envelope.message_type == message_type).count()
}

/// Scenario: A copied identity folder. Machine B started last, so its heartbeat writes the shared row: A finds `mac-b`
/// there, skips its offers, logs one error and reports `Duplicate` each round; B finds its own name and offers. The pool
/// marks the node with A's report and alerts once.
#[tokio::test(start_paused = true)]
async fn the_member_that_finds_another_host_skips_its_offers_logs_once_and_reports_the_duplicate() {
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || writer.clone()).finish();
    let _logging = tracing::subscriber::set_default(subscriber);

    let test = pool_node().await;
    let (first, copy) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let a_identity = NodeIdentity::load_or_create(first.path()).unwrap();
    fs::copy(first.path().join("placement-identity.json"), copy.path().join("placement-identity.json")).unwrap();
    let b_identity = NodeIdentity::load_or_create(copy.path()).unwrap();
    assert_eq!(a_identity, b_identity, "a copied folder: one node id and instance");

    let a = machine(&test, "mac-a").await;
    settle().await;
    let b = machine(&test, "mac-b").await;
    settle().await;
    assert_eq!(a.node(a.node_id()).await.unwrap().unwrap().hostname.as_deref(), Some("mac-b"), "B wrote last");
    let (stop_a, run_a) = member(&a, a_identity.clone());
    let (stop_b, run_b) = member(&b, b_identity);
    settle().await;
    for _ in 0..4 {
        test.advance_time_by(secs(15)).await;
        settle().await;
    }
    test.run_until_idle().await;

    let state: PlacementState = test.state(&pool()).await;
    let entry = &state.nodes["shared"];
    assert_eq!(entry.instance, a_identity.instance());
    let conflict = entry.conflict.clone().expect("marked");
    assert_eq!(conflict.host.as_deref(), Some("mac-b"), "A reports the host it found, B's");
    assert_eq!(conflict.instance, a_identity.instance(), "a mark set by reports names the node's own instance");
    assert_eq!(handled(&test, "Duplicate").await, 5, "one report per round of A, 0 through 60 s");
    assert_eq!(handled(&test, "Offer").await, 5, "B offers each round; A never does");
    let notices = test.state::<_, Notices>(&Ops).await.notices;
    let kinds: Vec<_> = notices.iter().map(|n| (n.node.as_str(), n.kind)).collect();
    assert_eq!(kinds, [("shared", PlacementNoticeKind::Conflict)], "one alert");
    assert_eq!(log.errors("another machine, 'mac-b', runs under this node id (shared)"), 1, "one error per duplicate");

    for (stop, run) in [(stop_a, run_a), (stop_b, run_b)] {
        stop.send(()).unwrap();
        run.await.unwrap();
    }
    a.stop().await;
    b.stop().await;
}

/// Scenario: A redeploy. The old node stops and removes its row; the new one, under a new host name, offers and
/// reports nothing.
#[tokio::test(start_paused = true)]
async fn a_redeploy_under_a_new_host_name_reports_nothing() {
    let test = pool_node().await;
    let temp = TempDir::new().unwrap();
    let old = machine(&test, "runner-old").await;
    settle().await;
    let (stop, run) = member(&old, NodeIdentity::load_or_create(temp.path()).unwrap());
    test.advance_time_by(secs(20)).await;
    stop.send(()).unwrap();
    run.await.unwrap();
    old.stop().await;

    let new = machine(&test, "runner-new").await;
    settle().await;
    let (stop, run) = member(&new, NodeIdentity::load_or_create(temp.path()).unwrap());
    for _ in 0..3 {
        test.advance_time_by(secs(15)).await;
        settle().await;
    }
    test.run_until_idle().await;
    assert_eq!(handled(&test, "Duplicate").await, 0);
    assert_eq!(handled(&test, "Offer").await, 6, "two rounds of the old node, four of the new");
    let state: PlacementState = test.state(&pool()).await;
    assert_eq!(state.nodes["shared"].conflict, None);
    stop.send(()).unwrap();
    run.await.unwrap();
    new.stop().await;
}
