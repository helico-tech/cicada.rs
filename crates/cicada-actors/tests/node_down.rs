//! The node-down monitor, on virtual time with `TestActors` and a scripted source: the first and repeated dead
//! samples, the two-minute boundary, new episodes between polls, absent and recovered rows without a message, failed,
//! slow, invalid and oversized reads that keep history, early, forged and redriven polls, `Rearm`, a topic that is gone
//! and its repair, two hosts of the singleton and a lost claim. And the source of an unstarted `ActorSystem` against
//! real node rows of the in-memory store.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::{NodeConfig, NodeHealth, NodeInfo, RetryResult};
use cicada::store::{Clock, MessageStatus, NodeId, OutcomeStatus};
use cicada::testing::{TokioClock, VirtualClock};
use cicada::time::{self, Timestamp};
use cicada::{FailureCause, Store};
use cicada_actors::{
    NodeDown, NodeDownMonitor, NodeDownMonitorActor, NodeDownMonitorMsg, NodeDownMonitorState, NodeDownWatermark,
    NodeReadError, NodeReadProblem, NodeSnapshot, NodeSnapshotSource, Topic, TopicActor, TopicMsg, TopicState,
};
use cicada_test::{TestActors, claim_and_abandon};
use common::*;
use serde::{Deserialize, Serialize};

const SECOND: Duration = Duration::from_secs(1);
const START: &str = "2026-01-01T00:00:00Z";

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn topic() -> Topic {
    Topic::named("nodes")
}

/// What the scripted source answers.
#[derive(Debug, Clone)]
enum Reply {
    /// These rows, sampled at the store time of the read.
    Rows(Vec<NodeInfo>),
    /// These rows, sampled at the given time.
    At(Timestamp, Vec<NodeInfo>),
    /// A failed read.
    Fail,
    /// A read that takes six seconds.
    Hang,
}

/// What the scripted source holds: its answer, its count of reads and the store clock of the test.
struct Script {
    reply: Reply,
    reads: usize,
    clock: Option<Arc<TokioClock>>,
}

/// A source that answers what the test set, and counts its reads. Its clones share the script.
#[derive(Clone)]
struct Scripted(Arc<Mutex<Script>>);

impl Scripted {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Script { reply: Reply::Rows(Vec::new()), reads: 0, clock: None })))
    }

    fn set(&self, reply: Reply) {
        self.0.lock().unwrap().reply = reply;
    }

    fn rows(&self, rows: Vec<NodeInfo>) {
        self.set(Reply::Rows(rows));
    }

    fn reads(&self) -> usize {
        self.0.lock().unwrap().reads
    }

    /// The store time of the test, truncated as the store hands it out.
    fn now(&self) -> Timestamp {
        time::truncate(self.0.lock().unwrap().clock.as_ref().expect("the clock is set").now())
    }
}

impl NodeSnapshotSource for Scripted {
    async fn snapshot(&self) -> Result<NodeSnapshot, NodeReadError> {
        let reply = {
            let mut script = self.0.lock().unwrap();
            script.reads += 1;
            script.reply.clone()
        };
        match reply {
            Reply::Rows(nodes) => Ok(NodeSnapshot { as_of: self.now(), nodes }),
            Reply::At(as_of, nodes) => Ok(NodeSnapshot { as_of, nodes }),
            Reply::Fail => Err(NodeReadError("the store is down".into())),
            Reply::Hang => {
                tokio::time::sleep(secs(6)).await;
                Ok(NodeSnapshot { as_of: self.now(), nodes: Vec::new() })
            }
        }
    }
}

/// A row of `id` that started at `started_at` and was last seen at `last_seen`, with a label.
fn row(id: &str, started_at: Timestamp, last_seen: Timestamp) -> NodeInfo {
    NodeInfo {
        id: NodeId::new(id).unwrap(),
        hosted: Vec::new(),
        started_at,
        last_seen,
        hostname: Some("web-1".into()),
        cicada_version: None,
        label: Some("shop 1.2".into()),
        status: NodeHealth::Alive,
        running_turns: Vec::new(),
    }
}

/// `sink/_`: keeps every `NodeDown` its topic delivers.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sink;

impl ActorRef for Sink {
    const TYPE: &'static str = "sink";
    type Message = SinkMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SinkMsg {
    NodeDown(NodeDown),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct SinkState {
    #[serde(default)]
    downs: Vec<NodeDown>,
}

struct SinkActor;

impl Actor for SinkActor {
    type Ref = Sink;
    type State = SinkState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SinkMsg) -> HandlerResult {
        let SinkMsg::NodeDown(down) = message;
        turn.state_mut().downs.push(down);
        Ok(())
    }
}

/// A node at `START` with the monitor on a scripted source, the topic, the sink and the witness, which watches the
/// monitor; the topic exists with the sink subscribed, the monitor exists but is not started.
async fn node() -> (TestActors, Scripted) {
    let source = Scripted::new();
    let hosted = source.clone();
    let test = TestActors::builder()
        .start_at(ts(START))
        .with(move |node| {
            node.host(NodeDownMonitorActor::new(hosted)).host(TopicActor).host(SinkActor).host(WitnessActor)
        })
        .build()
        .await
        .unwrap();
    source.0.lock().unwrap().clock = Some(test.clock().clone());
    test.create(&topic(), &TopicState::default()).await;
    test.create(&Sink, &SinkState::default()).await;
    test.send(&topic(), TopicMsg::Subscribe { subscriber: addr(&Sink) }).await;
    test.create(&NodeDownMonitor, &NodeDownMonitorState::new(topic()).unwrap()).await;
    witness(&test, "cicada.node-down").await;
    test.run_until_idle().await;
    (test, source)
}

/// [`node`], started with `Rearm`, after the source was set to `rows`: the first poll ran.
async fn started(rows: Vec<NodeInfo>) -> (TestActors, Scripted) {
    let (test, source) = node().await;
    source.rows(rows);
    send(&test, NodeDownMonitorMsg::Rearm).await;
    (test, source)
}

async fn send(test: &TestActors, message: NodeDownMonitorMsg) -> cicada::store::Message {
    let m = test.send(&NodeDownMonitor, message).await;
    test.run_until_idle().await;
    m
}

async fn held(test: &TestActors) -> NodeDownMonitorState {
    test.state(&NodeDownMonitor).await
}

async fn downs(test: &TestActors) -> Vec<NodeDown> {
    test.state::<_, SinkState>(&Sink).await.downs
}

/// The node ids of the alerts so far.
async fn alerted_ids(test: &TestActors) -> Vec<String> {
    downs(test).await.into_iter().map(|d| d.node.to_string()).collect()
}

/// The polls waiting in the monitor's mailbox; asserts that there is one and it is the armed one.
async fn armed(test: &TestActors) -> cicada::store::Message {
    let pending: Vec<_> =
        test.pending(&NodeDownMonitor).await.into_iter().filter(|m| m.message_type == "Poll").collect();
    assert_eq!(pending.len(), 1, "{pending:?}");
    let state = held(test).await;
    assert_eq!(state.armed.as_ref(), Some(&pending[0].id), "the pending poll is the armed one");
    assert_eq!(state.next_poll_at, Some(pending[0].deliver_at), "nextPollAt is its due time");
    pending.into_iter().next().unwrap()
}

fn at(offset: Duration) -> Timestamp {
    time::add(ts(START), offset).unwrap()
}

/// `offset` before `START`.
fn before(offset: Duration) -> Timestamp {
    time::sub(ts(START), offset)
}

fn mark(started_at: Timestamp, last_seen: Timestamp) -> NodeDownWatermark {
    NodeDownWatermark { started_at, last_seen }
}

fn id(text: &str) -> NodeId {
    NodeId::new(text).unwrap()
}

// Starting and the first sample.

/// Scenario: A created monitor does nothing until `Rearm`; then it polls at once and every ten seconds after.
#[tokio::test(start_paused = true)]
async fn a_created_monitor_waits_for_rearm_then_polls_at_once_and_every_ten_seconds() {
    let (test, source) = node().await;
    test.advance_time_by(secs(60)).await;
    assert_eq!((source.reads(), held(&test).await), (0, NodeDownMonitorState::new(topic()).unwrap()));
    send(&test, NodeDownMonitorMsg::Rearm).await;
    assert_eq!(source.reads(), 1);
    let state = held(&test).await;
    assert_eq!((state.last_checked_at, state.next_poll_at), (Some(at(secs(60))), Some(at(secs(70)))));
    armed(&test).await;
    test.advance_time_by(secs(30)).await;
    assert_eq!(source.reads(), 4);
    assert_eq!(held(&test).await.last_checked_at, Some(at(secs(90))));
    assert!(told(&test).await.is_empty());
}

/// Scenario: First dead observation, also of a row never seen alive: one alert with the row's details and the sample
/// time, published through the topic; the same watermark alerts no more.
#[tokio::test(start_paused = true)]
async fn a_row_dead_at_the_first_sample_alerts_once_and_the_same_watermark_never_again() {
    let dead = row("n1", before(secs(600)), before(secs(300)));
    let (test, _source) = started(vec![dead.clone(), row("n2", before(secs(60)), ts(START))]).await;
    let expected = NodeDown {
        node: id("n1"),
        started_at: dead.started_at,
        last_seen: dead.last_seen,
        detected_at: ts(START),
        hostname: Some("web-1".into()),
        cicada_version: None,
        label: Some("shop 1.2".into()),
    };
    assert_eq!(downs(&test).await, [expected]);
    assert_eq!(
        held(&test).await.alerted.into_iter().collect::<Vec<_>>(),
        [(id("n1"), mark(dead.started_at, dead.last_seen))]
    );
    test.advance_time_by(secs(300)).await;
    assert_eq!(alerted_ids(&test).await, ["n1", "n2"], "n2 went dead later, n1 alerted once");
}

/// Scenario: The two-minute boundary is the node view's: 119.999999 seconds of silence is not dead, 120 is.
#[tokio::test(start_paused = true)]
async fn a_row_silent_for_120_seconds_is_dead_and_one_a_microsecond_less_is_not() {
    let almost = time::sub(at(secs(10)), secs(120) - Duration::from_micros(1));
    let exactly = time::sub(at(secs(10)), secs(120));
    let (test, _source) =
        started(vec![row("almost", before(secs(600)), almost), row("exactly", before(secs(600)), exactly)]).await;
    assert!(downs(&test).await.is_empty(), "neither is dead at the first sample");
    test.advance_time_by(secs(10)).await;
    assert_eq!(alerted_ids(&test).await, ["exactly"]);
    test.advance_time_by(secs(10)).await;
    assert_eq!(alerted_ids(&test).await, ["exactly", "almost"]);
}

// Episodes.

/// Scenario: A heartbeat and a new death between two polls, and a new row under the same id, are new episodes: each
/// alerts once, without an alive sample in between.
#[tokio::test(start_paused = true)]
async fn a_newer_last_seen_or_another_started_at_of_a_dead_row_alerts_again() {
    let started_at = before(secs(3600));
    let (test, source) = started(vec![row("n1", started_at, before(secs(300)))]).await;
    source.rows(vec![row("n1", started_at, before(secs(200)))]);
    test.advance_time_by(secs(10)).await;
    let restarted = before(secs(250));
    source.rows(vec![row("n1", restarted, before(secs(200)))]);
    test.advance_time_by(secs(10)).await;
    test.advance_time_by(secs(30)).await;
    let seen: Vec<_> = downs(&test).await.into_iter().map(|d| (d.started_at, d.last_seen)).collect();
    assert_eq!(
        seen,
        [(started_at, before(secs(300))), (started_at, before(secs(200))), (restarted, before(secs(200)))]
    );
    assert_eq!(held(&test).await.alerted[&id("n1")], mark(restarted, before(secs(200))));
}

/// Scenario: A row that comes back alive, or goes from the table (a clean stop or the daily pruning), leaves `alerted`
/// without any message; seen dead again with the same timestamps it is a new observation and alerts again.
#[tokio::test(start_paused = true)]
async fn an_absent_or_alive_row_leaves_alerted_without_a_message_and_a_later_death_alerts_again() {
    let dead = row("n1", before(secs(600)), before(secs(300)));
    let gone = row("n2", before(secs(600)), before(secs(300)));
    let (test, source) = started(vec![dead.clone(), gone.clone()]).await;
    assert_eq!(alerted_ids(&test).await, ["n1", "n2"]);
    source.rows(vec![row("n1", dead.started_at, at(secs(10)))]);
    test.advance_time_by(secs(10)).await;
    assert!(held(&test).await.alerted.is_empty(), "alive and absent rows are forgotten");
    assert_eq!(alerted_ids(&test).await, ["n1", "n2"], "no recovery message");
    source.rows(vec![gone.clone()]);
    test.advance_time_by(secs(10)).await;
    assert_eq!(alerted_ids(&test).await, ["n1", "n2", "n2"], "a row that was absent is observed again");
    assert!(told(&test).await.is_empty());
}

// Failed reads.

/// Scenario: Read fails then recovers. A failed, slow, invalid or oversized read publishes nothing, keeps `alerted`
/// and `lastCheckedAt`, records only its category and arms the next poll; a good read clears it and goes on from the
/// kept watermarks, without a recovery message.
#[tokio::test(start_paused = true)]
async fn a_failed_slow_invalid_or_oversized_read_keeps_history_records_its_category_and_polls_on() {
    let dead = row("n1", before(secs(600)), before(secs(300)));
    let (test, source) = started(vec![dead.clone()]).await;
    let kept = held(&test).await.alerted;
    let mut too_many: Vec<NodeInfo> =
        (0..1001).map(|i| row(&format!("m{i}"), before(secs(600)), before(secs(300)))).collect();
    too_many[0] = dead.clone();
    let cases = [
        (Reply::Fail, NodeReadProblem::Unavailable),
        (Reply::Hang, NodeReadProblem::Timeout),
        (
            Reply::Rows(vec![dead.clone(), row("n2", before(secs(60)), before(secs(200))), dead.clone()]),
            NodeReadProblem::Invalid,
        ),
        (Reply::Rows(vec![row("n1", dead.started_at, before(secs(301)))]), NodeReadProblem::Invalid),
        (Reply::At(before(SECOND), vec![]), NodeReadProblem::Invalid),
        (Reply::Rows(too_many), NodeReadProblem::TooManyNodes),
    ];
    let mut due = at(secs(10));
    for (reply, problem) in cases {
        source.set(reply.clone());
        let reads = source.reads();
        test.advance_time_by(time::between(test.now(), due)).await;
        let state = held(&test).await;
        assert_eq!(source.reads(), reads + 1, "{reply:?}");
        assert_eq!(state.last_problem, Some(problem), "{reply:?}");
        assert_eq!((&state.alerted, state.last_checked_at), (&kept, Some(ts(START))), "{reply:?}");
        let ended = if matches!(reply, Reply::Hang) { time::add(due, secs(5)).unwrap() } else { due };
        assert_eq!(state.next_poll_at, Some(time::add(ended, secs(10)).unwrap()), "{reply:?}");
        armed(&test).await;
        due = state.next_poll_at.unwrap();
    }
    assert_eq!(alerted_ids(&test).await, ["n1"], "nothing published while reads failed");
    source.rows(vec![dead.clone(), row("n3", before(secs(600)), before(secs(300)))]);
    test.advance_time_by(time::between(test.now(), due)).await;
    let state = held(&test).await;
    assert_eq!((state.last_problem, state.last_checked_at), (None, Some(due)));
    assert_eq!(alerted_ids(&test).await, ["n1", "n3"], "n1 kept its watermark through the failures");
    assert!(told(&test).await.is_empty(), "a caught read error is not a failed turn");
}

// The timer.

/// Scenario: Ignore or defer an invalid poll. A poll from a client, a superseded or redriven copy reads nothing and
/// changes nothing; the armed poll made due early arms itself again at the same time without a read.
#[tokio::test(start_paused = true)]
async fn forged_redriven_and_early_polls_read_nothing() {
    let (test, source) = started(vec![]).await;
    let first = armed(&test).await;
    let version = test.version(&NodeDownMonitor).await;
    let forged = send(&test, NodeDownMonitorMsg::Poll).await;
    assert_eq!(status(&test, &forged).await, Some(OutcomeStatus::Done));
    assert_eq!((source.reads(), test.version(&NodeDownMonitor).await), (1, version), "a forged poll changes nothing");

    test.system().deliver_now(&first.id).await.unwrap();
    test.run_until_idle().await;
    let early = armed(&test).await;
    assert_ne!(early.id, first.id);
    assert_eq!((source.reads(), early.deliver_at), (1, at(secs(10))), "re-armed at the same time without a read");

    send(&test, NodeDownMonitorMsg::Rearm).await;
    let current = armed(&test).await;
    assert_eq!(status(&test, &early).await, Some(OutcomeStatus::Cancelled), "superseded by the Rearm");
    let RetryResult::Accepted(copy) = test.system().retry_message(&early.id).await.unwrap() else {
        panic!("the cancelled poll can be redriven")
    };
    test.run_until_idle().await;
    assert_eq!(status(&test, &copy).await, Some(OutcomeStatus::Done));
    assert_eq!(armed(&test).await.id, current.id, "a redriven copy is not the armed poll");
    assert_eq!(source.reads(), 1);
    test.advance_time_by(secs(10)).await;
    assert_eq!(source.reads(), 2);
}

/// Scenario: Recover a lost timer. `Rearm` after the armed poll was cancelled arms one at the kept time, and keeps
/// `alerted`, so it repeats no alert; a `Rearm` while a poll is armed replaces it at the same time.
#[tokio::test(start_paused = true)]
async fn rearm_restores_a_cancelled_timer_at_its_time_and_keeps_alerted() {
    let (test, source) = started(vec![row("n1", before(secs(600)), before(secs(300)))]).await;
    let timer = armed(&test).await;
    test.system().cancel(&timer.id).await.unwrap();
    test.advance_time_by(secs(60)).await;
    assert_eq!(source.reads(), 1, "no timer, no poll");
    send(&test, NodeDownMonitorMsg::Rearm).await;
    assert_eq!(source.reads(), 2, "the kept time had passed: due at once");
    assert_eq!(alerted_ids(&test).await, ["n1"], "the watermark was kept");
    let before_rearm = armed(&test).await;
    send(&test, NodeDownMonitorMsg::Rearm).await;
    let replaced = armed(&test).await;
    assert_ne!(replaced.id, before_rearm.id);
    assert_eq!((replaced.deliver_at, source.reads()), (before_rearm.deliver_at, 2));
    assert!(test.pending(&NodeDownMonitor).await.len() == 1, "the old timer was cancelled");
}

// The topic.

/// Scenario: Competing hosts or failed publication. A topic that is gone rejects the commit of the whole check: no
/// alert, no watermark, no new timer, and the watch of the monitor is told. After the topic is repaired, `Rearm` polls
/// at once and the alert is delivered once.
#[tokio::test(start_paused = true)]
async fn a_missing_topic_rolls_the_check_back_and_repair_with_rearm_delivers_once() {
    let (test, source) = started(vec![]).await;
    let before_failure = held(&test).await;
    test.system().delete_actor(&addr(&topic())).await.unwrap();
    source.rows(vec![row("n1", before(secs(600)), before(secs(300)))]);
    test.advance_time_by(secs(10)).await;
    assert_eq!(held(&test).await, before_failure, "nothing of the check was kept");
    let failed: Vec<_> = told(&test).await.into_iter().filter(|f| f.actor == addr(&NodeDownMonitor)).collect();
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!((failed[0].message_type.as_str(), failed[0].cause), ("Poll", FailureCause::Commit));
    test.advance_time_by(secs(60)).await;
    assert_eq!(source.reads(), 2, "the timer is lost");

    test.create(&topic(), &TopicState::default()).await;
    test.send(&topic(), TopicMsg::Subscribe { subscriber: addr(&Sink) }).await;
    send(&test, NodeDownMonitorMsg::Rearm).await;
    test.advance_time_by(secs(30)).await;
    assert_eq!(alerted_ids(&test).await, ["n1"]);
    assert_eq!(held(&test).await.last_problem, None);
}

/// Scenario: Competing hosts. Two nodes on one store host the singleton with their own sources; they poll the same
/// mailbox under one lease, so the observed watermark is published once.
#[tokio::test(start_paused = true)]
async fn two_nodes_hosting_the_monitor_publish_an_observed_death_once() {
    let (test, source) = started(vec![]).await;
    let config = NodeConfig { node_id: id("other"), ..NodeConfig::default() };
    let other = ActorSystem::builder(test.store().clone())
        .host(NodeDownMonitorActor::new(source.clone()))
        .host(TopicActor)
        .host(SinkActor)
        .host(WitnessActor)
        .config(config)
        .build();
    other.initialize().await.unwrap();
    source.rows(vec![row("n1", before(secs(600)), before(secs(300)))]);
    for _ in 0..6 {
        tokio::time::sleep(secs(10)).await;
        loop {
            let (a, b) = tokio::join!(test.system().run_due_turns(4), other.run_due_turns(4));
            if a.unwrap() + b.unwrap() == 0 {
                break;
            }
        }
    }
    assert_eq!(source.reads(), 7, "one read per poll, whichever node ran it");
    assert_eq!(alerted_ids(&test).await, ["n1"]);
}

/// Scenario: Competing hosts or failed publication, after a lost commit. A poll whose claim was abandoned commits
/// nothing; the core runs it again under the same id after the lease, and the alert is published once.
#[tokio::test(start_paused = true)]
async fn a_poll_whose_claim_was_abandoned_is_replayed_and_publishes_once() {
    let (test, source) = started(vec![]).await;
    source.rows(vec![row("n1", before(secs(600)), before(secs(300)))]);
    let timer = armed(&test).await;
    tokio::time::sleep(secs(10)).await;
    let lease = secs(30);
    let types = types::<NodeDownMonitor>();
    assert_eq!(claim_and_abandon(test.store(), &types, lease, 1).await.unwrap().len(), 1);
    assert!(downs(&test).await.is_empty(), "nothing committed");
    tokio::time::sleep(lease + SECOND).await;
    test.run_until_idle().await;
    assert_eq!(alerted_ids(&test).await, ["n1"]);
    let Some(MessageStatus::Finished(outcome)) = test.system().status(&timer.id).await.unwrap() else {
        panic!("the replayed poll finished")
    };
    assert_eq!((outcome.status, outcome.envelope.attempt), (OutcomeStatus::Done, 2));
}

// The source of an ActorSystem.

/// Scenario: Construct without a circular dependency. An unstarted system on the store reads the real node rows
/// without a row of its own; the monitor it feeds sees a stopped heartbeat as a death after two minutes and a clean
/// stop as an absence. Real time for the heartbeat, which does not fire again within the test; store time is a
/// `VirtualClock`, moved by hand.
#[tokio::test]
async fn an_unstarted_system_reads_the_real_node_rows_without_a_row_of_its_own() {
    let clock = Arc::new(VirtualClock::new(ts(START)));
    let store = Store::memory_with(clock.clone(), time::DEFAULT_RETENTION);
    let config = NodeConfig { node_id: id("web-1"), label: Some("shop".into()), ..NodeConfig::default() };
    let web = ActorSystem::builder(store.clone()).config(config).build();
    web.start().await.unwrap();
    while web.nodes().await.unwrap().is_empty() {
        tokio::time::sleep(Duration::from_millis(1)).await; // the first heartbeat runs in a task of its own
    }

    let reader = ActorSystem::builder(store.clone()).build();
    let host = ActorSystem::builder(store.clone())
        .host(NodeDownMonitorActor::new(reader.clone()))
        .host(TopicActor)
        .host(SinkActor)
        .build();
    host.initialize().await.unwrap();
    let snapshot = reader.snapshot().await.unwrap();
    let ids: Vec<_> = snapshot.nodes.iter().map(|n| n.id.to_string()).collect();
    assert_eq!((ids, snapshot.as_of), (vec!["web-1".to_owned()], ts(START)), "only the started node has a row");

    host.create(&topic(), &TopicState::default()).await.unwrap();
    host.create(&Sink, &SinkState::default()).await.unwrap();
    host.send(&topic(), TopicMsg::Subscribe { subscriber: addr(&Sink) }).await.unwrap();
    host.create(&NodeDownMonitor, &NodeDownMonitorState::new(topic()).unwrap()).await.unwrap();
    host.send(&NodeDownMonitor, NodeDownMonitorMsg::Rearm).await.unwrap();
    let run = || async { while host.run_due_turns(8).await.unwrap() > 0 {} };
    run().await;
    let sink = |host: ActorSystem| async move {
        let state = host.get_actor(&addr(&Sink)).await.unwrap().unwrap().state;
        serde_json::from_value::<SinkState>(state).unwrap().downs
    };
    assert!(sink(host.clone()).await.is_empty(), "alive");

    clock.advance_by(secs(120));
    run().await;
    let downs = sink(host.clone()).await;
    assert_eq!(downs.len(), 1, "{downs:?}");
    let down = &downs[0];
    assert_eq!((down.node.as_str(), down.last_seen, down.detected_at), ("web-1", ts(START), at(secs(120))));
    assert_eq!(
        (down.label.as_deref(), down.cicada_version.as_deref()),
        (Some("shop"), Some(env!("CARGO_PKG_VERSION")))
    );

    web.stop().await;
    clock.advance_by(secs(11)); // the next poll counts from the end of the last, in real time
    run().await;
    let state = host.get_actor(&addr(&NodeDownMonitor)).await.unwrap().unwrap().state;
    let state: NodeDownMonitorState = serde_json::from_value(state).unwrap();
    assert!(state.alerted.is_empty(), "a clean stop is an absence");
    assert_eq!(sink(host.clone()).await.len(), 1, "and tells nobody");
    assert!(reader.nodes().await.unwrap().is_empty(), "the reader and the host never wrote a row");
}
