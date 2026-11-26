//! The placement pool, on virtual time with `TestActors`: offers that renew and replace, labels sorted and each once,
//! silence with one notice and the return with `back`, the check armed at the earliest end of a silence, a lost check
//! and `Rearm`, a `Check` or `Alert` from another sender, a failing alert target, a pool without a target, and the
//! bounds of an offer and of the pool. And asks (ADR 0068): placement by needs and free places, refusals, the line that
//! skips asks it cannot place, the holder limit, releases and cancels, repeated asks, grants from the line and the
//! grant nobody received. And holders that go away: the pool watches each target with `watch_existing` while it
//! holds or waits, and a deleted target frees its places and leaves the line. And an operator's drain, undrain and gone:
//! a draining node gets no new places and keeps its holders, and a silent node declared gone tells each holder once.
//! And a second machine under one node id (ADR 0069): its offers are ignored, the node is marked in conflict with one
//! alert, and the mark ends when the node goes silent or the other machine stops. And a member's `Duplicate` report
//! (ADR 0071): the mark with the other host, one alert, no new places while marked, and the end one silence time after
//! the last report; a report of a silent node changes nothing and one of an unknown node is refused.
//! And the operator's `cicada send` recipes for `Drain`, `Undrain`, `Gone` and `Release`, over the HTTP send route.

mod common;

use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::RetryResult;
use cicada::store::{Message, OutcomeStatus};
use cicada::time::{self, Timestamp};
use cicada::{Failed, FailureCause, StopCause, Stopped};
use cicada_actors::{
    Placement, PlacementActor, PlacementAnswer, PlacementAnswerKind, PlacementConflict, PlacementMsg,
    PlacementNodeStatus, PlacementNotice, PlacementNoticeKind, PlacementRefusal, PlacementState,
};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn pool() -> Placement {
    Placement::named("runners")
}

/// `ops/{id}`: keeps the notices of the pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ops {
    id: String,
}

impl ActorRef for Ops {
    const TYPE: &'static str = "ops";
    type Message = OpsMsg;
}

fn ops() -> Ops {
    Ops { id: "pager".into() }
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

/// `forger/_`: sends the pool what it is given, as another actor.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Forger;

impl ActorRef for Forger {
    const TYPE: &'static str = "forger";
    type Message = ForgerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ForgerMsg {
    Forward { message: PlacementMsg },
}

struct ForgerActor;

impl Actor for ForgerActor {
    type Ref = Forger;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ForgerMsg) -> HandlerResult {
        let ForgerMsg::Forward { message } = message;
        turn.send(&pool(), message)?;
        Ok(())
    }
}

/// `run/{id}`: keeps the answers of the pool to its asks.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Run {
    id: String,
}

impl ActorRef for Run {
    const TYPE: &'static str = "run";
    type Message = RunMsg;
}

fn run(id: &str) -> Run {
    Run { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum RunMsg {
    /// An answer of a placement pool.
    Placed(PlacementAnswer),
    /// Fails for good.
    Fail,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<PlacementAnswer>,
}

struct RunActor;

impl Actor for RunActor {
    type Ref = Run;
    type State = Answers;

    async fn handle(&self, turn: &mut Turn<Self>, message: RunMsg) -> HandlerResult {
        match message {
            RunMsg::Placed(answer) => turn.state_mut().answers.push(answer),
            RunMsg::Fail => return Err(HandlerError::permanent("asked to fail")),
        }
        Ok(())
    }
}

/// A node with the pool type, `ops/pager`, the run type, the forger and the witness, which watches the pools, and the
/// pool `runners` created with a silence of `silence` seconds, at most `max_nodes` nodes, at most `max_waiting` waiting
/// asks and `max_holders` holders, and its notices going to `ops/pager`.
async fn node_limits(silence: u64, max_nodes: u32, max_waiting: u32, max_holders: u32) -> TestActors {
    let test = TestActors::builder()
        .start_at("2026-01-01T00:00:00Z".parse().unwrap())
        .with(|node| node.host(PlacementActor).host(OpsActor).host(ForgerActor).host(WitnessActor).host(RunActor))
        .build()
        .await
        .unwrap();
    test.create(&ops(), &Notices::default()).await;
    test.create(&Forger, &NoState {}).await;
    witness(&test, "cicada.placement").await;
    let state = PlacementState::new(secs(silence), max_nodes, max_waiting, max_holders).unwrap();
    test.create(&pool(), &state).await;
    send(&test, PlacementMsg::alerts(&ops(), "Placement").unwrap()).await;
    test
}

async fn node_with(silence: u64, max_nodes: u32) -> TestActors {
    node_limits(silence, max_nodes, 10, 10).await
}

async fn node() -> TestActors {
    node_with(60, 10).await
}

/// Sends the pool `message` from a client and runs what is due.
async fn send(test: &TestActors, message: PlacementMsg) -> Message {
    let m = test.send(&pool(), message).await;
    test.run_until_idle().await;
    m
}

async fn offer(test: &TestActors, node: &str, labels: &[&str], places: u32) -> Message {
    send(test, PlacementMsg::offer(node, "i1", labels.iter().copied(), places).unwrap()).await
}

async fn held(test: &TestActors) -> PlacementState {
    test.state(&pool()).await
}

/// The notices `ops/pager` got, as node and kind.
async fn notices(test: &TestActors) -> Vec<(String, PlacementNoticeKind)> {
    let notices = test.state::<_, Notices>(&ops()).await.notices;
    assert!(notices.iter().all(|n| n.pool == "runners"), "{notices:?}");
    notices.into_iter().map(|n| (n.node, n.kind)).collect()
}

/// The checks waiting in the mailbox of the pool, as offsets of their due times from `start`.
async fn checks(test: &TestActors, start: Timestamp) -> Vec<Duration> {
    let pending = test.pending(&pool()).await;
    pending.iter().filter(|m| m.message_type == "Check").map(|m| time::between(start, m.deliver_at)).collect()
}

/// The `Failed` the witness was told of the pool, as message type and cause.
async fn failed(test: &TestActors) -> Vec<(String, FailureCause)> {
    let told: Vec<Failed> = told(test).await.into_iter().filter(|f| f.actor == addr(&pool())).collect();
    told.into_iter().map(|f| (f.message_type, f.cause)).collect()
}

fn status_of(state: &PlacementState, node: &str) -> PlacementNodeStatus {
    state.nodes[node].status
}

/// Scenario: A renewed offer.
#[tokio::test(start_paused = true)]
async fn an_offer_replaces_the_last_one_and_records_when_the_node_was_heard() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 4).await;
    test.advance_time_by(secs(15)).await;
    let renewal = PlacementMsg::Offer {
        node: "mac-1".into(),
        instance: "i1".into(),
        labels: vec!["docker".into(), "arm64".into(), "docker".into()],
        places: 2,
        resource: None,
    };
    send(&test, renewal).await;
    let state = held(&test).await;
    let entry = &state.nodes["mac-1"];
    assert_eq!(
        (entry.instance.as_str(), entry.labels.clone(), entry.places),
        ("i1", vec!["arm64".into(), "docker".into()], 2)
    );
    assert_eq!((entry.heard_at, entry.status), (time::add(start, secs(15)).unwrap(), PlacementNodeStatus::Live));
    assert_eq!(state.armed, time::add(start, secs(60)).ok(), "armed at the first offer, not moved by the renewal");
    assert_eq!(checks(&test, start).await, [secs(60)], "one check");
    test.advance_time_by(secs(45)).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Live, "heard 45 s before the check");
    assert_eq!(checks(&test, start).await, [secs(75)], "the early check armed the next at the end of the silence");
    offer(&test, "mac-2", &[], 0).await;
    assert_eq!(status_of(&held(&test).await, "mac-2"), PlacementNodeStatus::Live, "no places: known but full");
    assert!(notices(&test).await.is_empty() && failed(&test).await.is_empty());
}

/// Scenario: Silence and return. A node quiet over three checks gets one `silent` notice, its next offer one `back`.
#[tokio::test(start_paused = true)]
async fn a_quiet_node_is_silent_once_and_an_offer_brings_it_back() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 4).await;
    offer(&test, "mac-2", &["x86_64"], 4).await;
    let mut seen: Vec<Message> = Vec::new();
    for _ in 0..12 {
        seen.extend(test.pending(&pool()).await.into_iter().filter(|m| m.message_type == "Check"));
        test.advance_time_by(secs(15)).await;
        offer(&test, "mac-2", &["x86_64"], 4).await;
    }
    let state = held(&test).await;
    assert_eq!(status_of(&state, "mac-1"), PlacementNodeStatus::Silent);
    assert_eq!(status_of(&state, "mac-2"), PlacementNodeStatus::Live);
    assert_eq!(state.nodes["mac-1"].heard_at, start, "a silent node keeps its last offer");
    assert_eq!(notices(&test).await, [("mac-1".to_owned(), PlacementNoticeKind::Silent)], "one notice over the checks");
    seen.dedup_by(|a, b| a.id == b.id);
    let mut ran = 0;
    for check in &seen {
        ran += usize::from(status(&test, check).await == Some(OutcomeStatus::Done));
    }
    assert!(ran >= 3, "at least three checks ran, {ran}");

    offer(&test, "mac-1", &["arm64"], 4).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Live);
    assert_eq!(
        notices(&test).await,
        [("mac-1".to_owned(), PlacementNoticeKind::Silent), ("mac-1".to_owned(), PlacementNoticeKind::Back)]
    );
    test.advance_time_by(secs(60)).await;
    let told = notices(&test).await;
    assert!(told[2..].contains(&("mac-1".to_owned(), PlacementNoticeKind::Silent)), "the next silence is told again");
    assert!(failed(&test).await.is_empty());
}

/// Once no node is live the pool arms nothing; the next offer arms a check at the end of its silence.
#[tokio::test(start_paused = true)]
async fn a_pool_without_live_nodes_arms_no_check_until_an_offer() {
    let test = node().await;
    let start = test.now();
    assert!(checks(&test, start).await.is_empty(), "a new pool arms nothing");
    offer(&test, "mac-1", &[], 1).await;
    test.advance_time_by(secs(60)).await;
    let state = held(&test).await;
    assert_eq!((status_of(&state, "mac-1"), state.armed), (PlacementNodeStatus::Silent, None));
    assert!(checks(&test, start).await.is_empty());
    test.advance_time_by(secs(40)).await;
    offer(&test, "mac-1", &[], 1).await;
    assert_eq!(checks(&test, start).await, [secs(160)]);
    assert_eq!(held(&test).await.armed, time::add(start, secs(160)).ok());
}

/// Scenario: A lost check. A purged check leaves the pool without a timer; `Rearm` arms it at the old time, at once
/// when that passed, and the silent node is found.
#[tokio::test(start_paused = true)]
async fn a_rearm_brings_back_a_purged_check() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 4).await;
    test.system().purge_mailbox(&addr(&pool())).await.unwrap();
    test.advance_time_by(secs(100)).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Live, "nobody checked");
    assert_eq!(held(&test).await.armed, time::add(start, secs(60)).ok(), "the old time is kept");
    send(&test, PlacementMsg::Rearm).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Silent);
    assert_eq!(notices(&test).await, [("mac-1".to_owned(), PlacementNoticeKind::Silent)]);
    assert_eq!(held(&test).await.armed, None);
    let m = send(&test, PlacementMsg::Rearm).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(checks(&test, start).await.is_empty(), "nothing to arm without a live node");
}

/// The one pending check, which is the armed one.
async fn armed_check(test: &TestActors) -> Message {
    let pending: Vec<Message> = test.pending(&pool()).await.into_iter().filter(|m| m.message_type == "Check").collect();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(held(test).await.armed_id.as_ref(), Some(&pending[0].id), "the pending check is the armed one");
    pending.into_iter().next().unwrap()
}

/// Repeated `Rearm`s while the check is still there each replace it, at the same time: one chain stays.
#[tokio::test(start_paused = true)]
async fn a_rearm_beside_a_live_check_replaces_it() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &[], 1).await;
    let first = armed_check(&test).await;
    test.advance_time_by(secs(30)).await;
    offer(&test, "mac-2", &[], 1).await;
    for _ in 0..3 {
        send(&test, PlacementMsg::Rearm).await;
    }
    assert_eq!(status(&test, &first).await, Some(OutcomeStatus::Cancelled), "the check before it");
    assert_eq!(checks(&test, start).await, [secs(60)]);
    armed_check(&test).await;
    test.advance_time_by(secs(30)).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Silent);
    assert_eq!(checks(&test, start).await, [secs(90)], "one check armed for mac-2");
    test.advance_time_by(secs(30)).await;
    assert!(checks(&test, start).await.is_empty());
    let silent = |n: &str| (n.to_owned(), PlacementNoticeKind::Silent);
    assert_eq!(notices(&test).await, [silent("mac-1"), silent("mac-2")]);
}

/// The armed check made due early by an operator marks nothing and arms itself again at the unchanged time, so a quiet
/// node is still found.
#[tokio::test(start_paused = true)]
async fn a_check_made_due_early_arms_itself_again_at_the_same_time() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &[], 1).await;
    let early = armed_check(&test).await;
    test.advance_time_by(secs(10)).await;
    test.system().deliver_now(&early.id).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(status(&test, &early).await, Some(OutcomeStatus::Done));
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Live, "nothing is due yet");
    let again = armed_check(&test).await;
    assert_ne!(again.id, early.id);
    assert_eq!(checks(&test, start).await, [secs(60)], "the same time");
    test.advance_time_by(secs(600)).await;
    let state = held(&test).await;
    assert_eq!((status_of(&state, "mac-1"), state.armed, state.armed_id), (PlacementNodeStatus::Silent, None, None));
    assert_eq!(notices(&test).await, [("mac-1".to_owned(), PlacementNoticeKind::Silent)]);
}

/// An operator's retry of a cancelled check comes under a new id and without a sender: it is refused for good and
/// changes nothing.
#[tokio::test(start_paused = true)]
async fn a_retried_check_is_refused_and_changes_nothing() {
    let test = node().await;
    offer(&test, "mac-1", &[], 1).await;
    let cancelled = armed_check(&test).await;
    send(&test, PlacementMsg::Rearm).await;
    assert_eq!(status(&test, &cancelled).await, Some(OutcomeStatus::Cancelled));
    test.advance_time_by(secs(70)).await;
    offer(&test, "mac-1", &[], 1).await;
    let (before, version, pending) = (held(&test).await, test.version(&pool()).await, test.pending(&pool()).await);
    let RetryResult::Accepted(copy) = test.system().retry_message(&cancelled.id).await.unwrap() else {
        panic!("not retried")
    };
    test.run_until_idle().await;
    assert_eq!(status(&test, &copy).await, Some(OutcomeStatus::Failed));
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version));
    assert_eq!(test.pending(&pool()).await, pending, "no check armed or cancelled");
    assert_eq!(failed(&test).await, [("Check".to_owned(), FailureCause::Refused)]);
}

/// A `Check` or an `Alert` from a client or another actor is refused for good and changes nothing.
#[tokio::test(start_paused = true)]
async fn a_check_or_alert_from_another_sender_is_refused() {
    let test = node().await;
    offer(&test, "mac-1", &[], 1).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    let alert = PlacementMsg::Alert { node: "mac-1".into(), kind: PlacementNoticeKind::Silent };
    for message in [PlacementMsg::Check, alert] {
        let m = send(&test, message.clone()).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed), "{message:?} from a client");
        test.send(&Forger, ForgerMsg::Forward { message }).await;
        test.run_until_idle().await;
    }
    let refused = |t: &str| (t.to_owned(), FailureCause::Refused);
    assert_eq!(failed(&test).await, [refused("Check"), refused("Check"), refused("Alert"), refused("Alert")]);
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version));
    assert!(notices(&test).await.is_empty());
}

/// Scenario: A failing alert target. A target that does not take the notice fails only the `Alert`; the node is
/// silent all the same, and a later target gets the next notice.
#[tokio::test(start_paused = true)]
async fn a_failing_alert_target_fails_only_its_alert() {
    let test = node().await;
    send(&test, PlacementMsg::alerts(&ops(), "Unknown").unwrap()).await;
    offer(&test, "mac-1", &[], 1).await;
    test.advance_time_by(secs(60)).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Silent);
    assert_eq!(failed(&test).await, [("Alert".to_owned(), FailureCause::Refused)]);
    assert!(notices(&test).await.is_empty());
    send(&test, PlacementMsg::alerts(&ops(), "Placement").unwrap()).await;
    offer(&test, "mac-1", &[], 1).await;
    assert_eq!(notices(&test).await, [("mac-1".to_owned(), PlacementNoticeKind::Back)], "the last Alerts stands");
}

/// A pool without an alert target sends no notices; its alert ends done.
#[tokio::test(start_paused = true)]
async fn a_pool_without_a_target_marks_silence_and_tells_nobody() {
    let test = node().await;
    let quiet = Placement::named("quiet");
    test.create(&quiet, &PlacementState::new(secs(10), 1, 0, 1).unwrap()).await;
    test.send(&quiet, PlacementMsg::offer("mac-1", "i1", ["arm64"], 1).unwrap()).await;
    test.run_until_idle().await;
    test.advance_time_by(secs(10)).await;
    let state: PlacementState = test.state(&quiet).await;
    assert_eq!((status_of(&state, "mac-1"), state.alerts), (PlacementNodeStatus::Silent, None));
    assert!(test.failures().await.is_empty(), "the alert ended done");
    assert!(test.pending(&quiet).await.is_empty() && notices(&test).await.is_empty());
}

/// Out-of-bounds offers and a new node past `maxNodes` are refused for good with the state unchanged; a known node may
/// still renew in a full pool.
#[tokio::test(start_paused = true)]
async fn offers_out_of_bounds_and_a_node_past_the_limit_are_refused() {
    let test = node_with(60, 2).await;
    offer(&test, "mac-1", &[], 1).await;
    offer(&test, "mac-2", &[], 1).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    let raw = |node: &str, instance: &str, labels: Vec<String>, places: u32| PlacementMsg::Offer {
        node: node.into(),
        instance: instance.into(),
        labels,
        places,
        resource: None,
    };
    let keyed = |resource: String| PlacementMsg::Offer {
        node: "mac-1".into(),
        instance: "i1".into(),
        labels: vec![],
        places: 1,
        resource: Some(resource),
    };
    let invalid = [
        keyed(String::new()),
        keyed("k".repeat(201)),
        raw("mac-3", "i1", vec![], 1),
        raw("", "i1", vec![], 1),
        raw(" ", "i1", vec![], 1),
        raw(&"n".repeat(201), "i1", vec![], 1),
        raw("mac-1", "", vec![], 1),
        raw("mac-1", &"i".repeat(201), vec![], 1),
        raw("mac-1", "i1", (0..33).map(|n| format!("l{n}")).collect(), 1),
        raw("mac-1", "i1", vec!["".into()], 1),
        raw("mac-1", "i1", vec!["l".repeat(101)], 1),
        raw("mac-1", "i1", vec![], 1001),
    ];
    let count = invalid.len();
    for message in invalid {
        let m = send(&test, message.clone()).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed), "{message:?}");
    }
    assert_eq!(failed(&test).await, vec![("Offer".to_owned(), FailureCause::Refused); count]);
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version), "nothing recorded");
    let m = offer(&test, "mac-1", &["arm64"], 3).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "a known node renews in a full pool");
    for (message, why) in [
        (PlacementMsg::Alerts { to: addr(&ops()), message_type: String::new() }, "an empty message type"),
        (PlacementMsg::Alert { node: String::new(), kind: PlacementNoticeKind::Back }, "an empty node"),
    ] {
        let m = send(&test, message).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed), "{why}");
    }
}

/// Creates `run/{holder}` when it does not exist and asks the pool a place for `holder` with `needs`, its answers going
/// to `run/{holder}`.
async fn ask(test: &TestActors, holder: &str, needs: &[&str]) -> Message {
    if test.state_json(&run(holder)).await.is_none() {
        test.create(&run(holder), &Answers::default()).await;
    }
    send(test, PlacementMsg::ask(holder, needs.iter().copied(), &run(holder), "Placed").unwrap()).await
}

async fn release(test: &TestActors, holder: &str, grant: u64) -> Message {
    send(test, PlacementMsg::release(holder, grant).unwrap()).await
}

/// What `run/{holder}` was told: kind, grant, node and reason.
async fn answers(test: &TestActors, holder: &str) -> Vec<(PlacementAnswerKind, Option<u64>, Option<String>)> {
    let answers = test.state::<_, Answers>(&run(holder)).await.answers;
    assert!(answers.iter().all(|a| a.pool == "runners" && a.holder == holder), "{answers:?}");
    answers
        .into_iter()
        .map(|a| {
            assert_eq!(a.reason.is_some(), a.kind == PlacementAnswerKind::Refused, "{a:?}");
            (a.kind, a.grant, a.node)
        })
        .collect()
}

/// The reasons of the refusals `run/{holder}` was told.
async fn reasons(test: &TestActors, holder: &str) -> Vec<PlacementRefusal> {
    test.state::<_, Answers>(&run(holder)).await.answers.into_iter().filter_map(|a| a.reason).collect()
}

fn granted(grant: u64, node: &str) -> (PlacementAnswerKind, Option<u64>, Option<String>) {
    (PlacementAnswerKind::Granted, Some(grant), Some(node.to_owned()))
}

const QUEUED: (PlacementAnswerKind, Option<u64>, Option<String>) = (PlacementAnswerKind::Queued, None, None);
const REFUSED: (PlacementAnswerKind, Option<u64>, Option<String>) = (PlacementAnswerKind::Refused, None, None);

/// The holders of the pool as holder, grant and node.
async fn holders(test: &TestActors) -> Vec<(String, u64, String)> {
    held(test).await.holders.into_iter().map(|h| (h.holder, h.grant, h.node)).collect()
}

/// The holder ids waiting in the line, in order.
async fn line(test: &TestActors) -> Vec<String> {
    held(test).await.waiting.into_iter().map(|w| w.holder).collect()
}

fn holder(holder: &str, grant: u64, node: &str) -> (String, u64, String) {
    (holder.to_owned(), grant, node.to_owned())
}

/// Scenario: Most free places first. Ties go to the smaller node id; grant numbers go up.
#[tokio::test(start_paused = true)]
async fn a_grant_goes_to_the_node_with_the_most_free_places_then_the_smaller_id() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    offer(&test, "mac-3", &["arm64", "docker"], 3).await;
    offer(&test, "mac-2", &["arm64"], 3).await;
    offer(&test, "x86-1", &["x86_64"], 9).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    ask(&test, "r3", &["docker", "arm64", "docker"]).await;
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-2")], "3 free on mac-2 and mac-3: the smaller id");
    assert_eq!(answers(&test, "r2").await, [granted(2, "mac-3")], "mac-2 has 2 free now");
    assert_eq!(answers(&test, "r3").await, [granted(3, "mac-3")], "only mac-3 has docker");
    assert_eq!(holders(&test).await, [holder("r1", 1, "mac-2"), holder("r2", 2, "mac-3"), holder("r3", 3, "mac-3")]);
    let state = held(&test).await;
    assert_eq!(state.next_grant, 4);
    assert_eq!(state.holders[2].needs, ["arm64", "docker"], "needs sorted, each once");
    assert!(state.holders.iter().all(|h| h.notified), "told in the ask's commit");
    assert!(failed(&test).await.is_empty());
}

/// Scenario: No node has the needs. Nothing is recorded, also when no node is known at all.
#[tokio::test(start_paused = true)]
async fn an_ask_no_node_could_meet_is_refused_with_no_node() {
    let test = node().await;
    ask(&test, "r0", &["arm64"]).await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    let m = ask(&test, "r1", &["arm64", "gpu"]).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(
        (answers(&test, "r0").await, reasons(&test, "r0").await),
        (vec![REFUSED], vec![PlacementRefusal::NoNode])
    );
    assert_eq!(
        (answers(&test, "r1").await, reasons(&test, "r1").await),
        (vec![REFUSED], vec![PlacementRefusal::NoNode])
    );
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version), "nothing recorded");
}

/// Scenario: A full line. A place frees and the next in line is granted, told in a turn of its own.
#[tokio::test(start_paused = true)]
async fn a_full_line_refuses_and_a_release_grants_the_next() {
    let test = node_limits(60, 10, 1, 10).await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    ask(&test, "r3", &["arm64"]).await;
    assert_eq!(
        (answers(&test, "r3").await, reasons(&test, "r3").await),
        (vec![REFUSED], vec![PlacementRefusal::LineFull])
    );
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version), "nothing recorded");
    assert_eq!(answers(&test, "r2").await, [QUEUED]);
    test.send(&pool(), PlacementMsg::release("r1", 1).unwrap()).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    let state = held(&test).await;
    assert_eq!(holders(&test).await, [holder("r2", 2, "mac-1")], "granted in the release's commit");
    assert!(!state.holders[0].notified && state.waiting.is_empty());
    test.run_until_idle().await;
    assert_eq!(answers(&test, "r2").await, [QUEUED, granted(2, "mac-1")]);
    assert!(held(&test).await.holders[0].notified);
}

/// A pool that holds `maxHolders` queues an ask that fits, and refuses it with `poolFull` when the line is full.
#[tokio::test(start_paused = true)]
async fn a_pool_at_its_holder_limit_queues_and_then_refuses_with_pool_full() {
    let test = node_limits(60, 10, 1, 1).await;
    offer(&test, "mac-1", &["arm64"], 3).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    ask(&test, "r3", &["arm64"]).await;
    assert_eq!(answers(&test, "r2").await, [QUEUED], "a place fits but the pool holds maxHolders");
    assert_eq!(
        (answers(&test, "r3").await, reasons(&test, "r3").await),
        (vec![REFUSED], vec![PlacementRefusal::PoolFull])
    );
    assert_eq!((holders(&test).await, line(&test).await), (vec![holder("r1", 1, "mac-1")], vec!["r2".to_owned()]));
    release(&test, "r1", 1).await;
    assert_eq!(answers(&test, "r2").await, [QUEUED, granted(2, "mac-1")]);
}

/// Scenario: An ask to a gone target. With a self-watch in place, its `Failed` changes nothing either.
#[tokio::test(start_paused = true)]
async fn an_ask_to_a_gone_target_fails_and_records_nothing() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    release(&test, "r1", 1).await;
    let watches = test.system().watches(Some(&addr(&pool())), None, None, 10).await.unwrap().items;
    let targets: Vec<String> = watches.iter().map(|w| w.target.to_string()).collect();
    assert_eq!(targets, ["cicada.placement/runners", "run/r2"], "itself once it sent a Notify, and its holder");
    for (ghost, needs) in [("ghost-1", vec!["arm64"]), ("ghost-2", vec!["arm64"]), ("ghost-3", vec!["gpu"])] {
        let (before, version) = (held(&test).await, test.version(&pool()).await);
        let m = send(&test, PlacementMsg::ask(ghost, needs, &run(ghost), "Placed").unwrap()).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed), "{ghost}");
        assert_eq!(failed(&test).await.last(), Some(&("Ask".to_owned(), FailureCause::Commit)));
        assert_eq!(held(&test).await, before, "{ghost}: nothing recorded");
        assert_eq!(test.version(&pool()).await, version, "{ghost}");
    }
    assert_eq!(holders(&test).await, [holder("r2", 2, "mac-1")]);
    assert!(line(&test).await.is_empty());
}

/// Out-of-bounds asks, releases and cancels, and a `Notify` from a client or another actor, are refused for good with
/// the state unchanged.
#[tokio::test(start_paused = true)]
async fn asks_releases_and_cancels_out_of_bounds_and_a_foreign_notify_are_refused() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    let raw = |holder: &str, needs: Vec<String>, message_type: &str| PlacementMsg::Ask {
        holder: holder.into(),
        needs,
        to: addr(&run("r1")),
        message_type: message_type.into(),
    };
    let invalid = [
        raw("", vec![], "Placed"),
        raw(&"h".repeat(201), vec![], "Placed"),
        raw("r9", (0..33).map(|n| format!("l{n}")).collect(), "Placed"),
        raw("r9", vec!["".into()], "Placed"),
        raw("r9", vec!["l".repeat(101)], "Placed"),
        raw("r9", vec![], ""),
        raw("r9", vec!["arm64".into()], "Unknown"),
        PlacementMsg::Release { holder: String::new(), grant: 1 },
        PlacementMsg::Cancel { holder: "h".repeat(201) },
        PlacementMsg::Notify { holder: String::new() },
        PlacementMsg::Notify { holder: "r2".into() },
        PlacementMsg::Notify { holder: "r1".into() },
    ];
    let count = invalid.len();
    for message in invalid {
        let m = send(&test, message.clone()).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed), "{message:?}");
    }
    test.send(&Forger, ForgerMsg::Forward { message: PlacementMsg::Notify { holder: "r2".into() } }).await;
    test.run_until_idle().await;
    let told = failed(&test).await;
    assert_eq!(told.len(), count + 1, "{told:?}");
    assert!(told.iter().all(|(_, cause)| *cause == FailureCause::Refused), "{told:?}");
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version), "nothing recorded");
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1")]);
    assert_eq!(answers(&test, "r2").await, [QUEUED]);
}

/// A new node, or an offer with a new label, grants a waiting ask that a full node could meet.
#[tokio::test(start_paused = true)]
async fn a_new_node_or_a_new_label_grants_a_waiting_ask() {
    let test = node().await;
    offer(&test, "gpu-1", &["gpu"], 0).await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["gpu"]).await;
    ask(&test, "r2", &["gpu"]).await;
    assert_eq!((answers(&test, "r1").await, line(&test).await), (vec![QUEUED], vec!["r1".to_owned(), "r2".to_owned()]));
    offer(&test, "gpu-2", &["gpu"], 1).await;
    assert_eq!(answers(&test, "r1").await, [QUEUED, granted(1, "gpu-2")], "a new node");
    offer(&test, "mac-1", &["arm64", "gpu"], 1).await;
    assert_eq!(answers(&test, "r2").await, [QUEUED, granted(2, "mac-1")], "a new label");
    assert!(line(&test).await.is_empty());
}

/// Scenario: Waiting for a node to come back.
#[tokio::test(start_paused = true)]
async fn an_ask_only_a_silent_node_could_meet_waits_until_it_offers_again() {
    let test = node().await;
    offer(&test, "gpu-1", &["gpu"], 1).await;
    test.advance_time_by(secs(60)).await;
    assert_eq!(status_of(&held(&test).await, "gpu-1"), PlacementNodeStatus::Silent);
    ask(&test, "r1", &["gpu"]).await;
    assert_eq!((answers(&test, "r1").await, line(&test).await), (vec![QUEUED], vec!["r1".to_owned()]));
    offer(&test, "gpu-1", &["gpu"], 1).await;
    assert_eq!(answers(&test, "r1").await, [QUEUED, granted(1, "gpu-1")]);
    test.advance_time_by(secs(60)).await;
    assert_eq!(holders(&test).await, [holder("r1", 1, "gpu-1")], "a silent node keeps its holders");
}

/// Scenario: A rare label does not block. Neither at the ask nor when a place frees.
#[tokio::test(start_paused = true)]
async fn an_ask_for_a_rare_label_does_not_hold_up_the_asks_behind_it() {
    let test = node().await;
    offer(&test, "gpu-1", &["gpu"], 0).await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "g1", &["gpu"]).await;
    ask(&test, "r1", &["arm64"]).await;
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1")], "granted at once past the waiting gpu ask");
    ask(&test, "r2", &["arm64"]).await;
    assert_eq!(line(&test).await, ["g1", "r2"]);
    release(&test, "r1", 1).await;
    assert_eq!(answers(&test, "r2").await, [QUEUED, granted(2, "mac-1")]);
    assert_eq!(line(&test).await, ["g1"], "the gpu ask keeps its place");
    offer(&test, "gpu-1", &[], 1).await;
    assert_eq!(line(&test).await, ["g1"], "a waiting ask whose needs no node has stays");
    let m = send(&test, PlacementMsg::cancel("g1").unwrap()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert!(line(&test).await.is_empty());
    assert_eq!(answers(&test, "g1").await, [QUEUED]);
}

/// Scenario: A late release. A release with another grant, or of a holder id not in the books, frees nothing.
#[tokio::test(start_paused = true)]
async fn a_release_with_an_older_grant_frees_nothing() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    ask(&test, "r1", &["arm64"]).await;
    release(&test, "r1", 1).await;
    ask(&test, "r1", &["arm64"]).await;
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1"), granted(2, "mac-1")]);
    for (holder, grant) in [("r1", 1), ("r1", 3), ("r1", 0), ("r9", 2)] {
        let m = release(&test, holder, grant).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    }
    assert_eq!(holders(&test).await, [holder("r1", 2, "mac-1")]);
}

/// A repeated ask of a holder answers the same grant and node; of a waiting ask, `queued` with its place kept. Both keep
/// the first ask's needs and target.
#[tokio::test(start_paused = true)]
async fn a_repeated_ask_is_answered_again_with_the_first_needs_and_target() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    offer(&test, "gpu-1", &["gpu"], 0).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    test.create(&run("other"), &Answers::default()).await;
    for holder in ["r1", "r2"] {
        send(&test, PlacementMsg::ask(holder, ["gpu"], &run("other"), "Placed").unwrap()).await;
    }
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1"), granted(1, "mac-1")]);
    assert_eq!(answers(&test, "r2").await, [QUEUED, QUEUED]);
    assert!(test.state::<_, Answers>(&run("other")).await.answers.is_empty(), "the first target is kept");
    let state = held(&test).await;
    assert_eq!((state.holders.len(), state.waiting.len(), state.next_grant), (1, 1, 2));
    assert_eq!(
        (state.holders[0].needs.clone(), state.waiting[0].needs.clone()),
        (vec!["arm64".into()], vec!["arm64".into()])
    );
}

/// The `Notify` failures the witness was told of the pool.
async fn failed_notifies(test: &TestActors) -> Vec<Failed> {
    told(test).await.into_iter().filter(|f| f.actor == addr(&pool()) && f.message_type == "Notify").collect()
}

/// Scenario: A grant nobody received. Its target is deleted after the grant's commit and before its `Notify`: the
/// place is freed once, by the `Stopped` or the failed `Notify`, and goes to the next ask that fits.
#[tokio::test(start_paused = true)]
async fn a_grant_from_the_line_to_a_gone_target_is_freed() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    for id in ["r1", "r2", "r3"] {
        ask(&test, id, &["arm64"]).await;
    }
    let m = test.send(&pool(), PlacementMsg::release("r1", 1).unwrap()).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "the release commits");
    assert_eq!(holders(&test).await, [holder("r2", 2, "mac-1")], "granted, not yet told");
    test.system().delete_actor(&addr(&run("r2"))).await.unwrap();
    test.run_until_idle().await;
    let failed = failed_notifies(&test).await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!((failed[0].cause, &failed[0].payload["holder"]), (FailureCause::Commit, &serde_json::json!("r2")));
    assert_eq!(holders(&test).await, [holder("r3", 3, "mac-1")]);
    assert!(line(&test).await.is_empty());
    assert_eq!(answers(&test, "r3").await, [QUEUED, granted(3, "mac-1")]);
}

/// An offer with fewer places than holders keeps every holder; new grants wait until fewer hold than the places.
#[tokio::test(start_paused = true)]
async fn fewer_offered_places_keep_the_holders() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r3", &["arm64"]).await;
    assert_eq!(holders(&test).await, [holder("r1", 1, "mac-1"), holder("r2", 2, "mac-1")]);
    assert_eq!(answers(&test, "r3").await, [QUEUED]);
    release(&test, "r1", 1).await;
    assert_eq!(line(&test).await, ["r3"], "one holder on one place");
    release(&test, "r2", 2).await;
    assert_eq!(answers(&test, "r3").await, [QUEUED, granted(3, "mac-1")]);
}

/// `Cancel` takes a waiting ask out of the line, which is never granted; for a holder it does nothing.
#[tokio::test(start_paused = true)]
async fn a_cancel_withdraws_a_waiting_ask_and_keeps_a_holder() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    for holder in ["r1", "r2", "r9"] {
        let m = send(&test, PlacementMsg::cancel(holder).unwrap()).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    }
    assert_eq!((holders(&test).await, line(&test).await), (vec![holder("r1", 1, "mac-1")], vec![]));
    release(&test, "r1", 1).await;
    assert!(holders(&test).await.is_empty());
    assert_eq!(answers(&test, "r2").await, [QUEUED]);
}

/// Asks the pool a place for `holder` with `needs`, its answers going to `run/{target}`, created when it does not exist.
async fn ask_for(test: &TestActors, holder: &str, needs: &[&str], target: &str) -> Message {
    if test.state_json(&run(target)).await.is_none() {
        test.create(&run(target), &Answers::default()).await;
    }
    send(test, PlacementMsg::ask(holder, needs.iter().copied(), &run(target), "Placed").unwrap()).await
}

/// The targets the pool watches, itself included.
async fn watched(test: &TestActors) -> Vec<String> {
    let watches = test.system().watches(Some(&addr(&pool())), None, None, 100).await.unwrap().items;
    let mut targets: Vec<String> = watches.iter().map(|w| w.target.to_string()).collect();
    targets.sort();
    targets
}

async fn delete(test: &TestActors, target: &str) {
    test.system().delete_actor(&addr(&run(target))).await.unwrap();
    test.run_until_idle().await;
}

/// The messages of `message_type` that the pool finished `done`.
async fn done(test: &TestActors, message_type: &str) -> usize {
    let page = test.system().outcomes(Some(OutcomeStatus::Done), "cicada.placement/", None, None, 500).await.unwrap();
    page.items.iter().filter(|o| o.envelope.message_type == message_type).count()
}

/// The pool watches a target from the commit that records its ask, granted or queued, and unwatches it after the last
/// release; a waiting ask turned holder keeps the watch.
#[tokio::test(start_paused = true)]
async fn the_pool_watches_the_targets_that_hold_or_wait_and_unwatches_after_the_last_release() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    assert_eq!(watched(&test).await, ["run/r1"], "granted at once");
    ask(&test, "r2", &["arm64"]).await;
    assert_eq!(watched(&test).await, ["run/r1", "run/r2"], "queued");
    release(&test, "r1", 1).await;
    assert_eq!(watched(&test).await, ["cicada.placement/runners", "run/r2"], "a waiting ask granted from the line");
    release(&test, "r2", 2).await;
    assert_eq!(watched(&test).await, ["cicada.placement/runners"], "the last release unwatches");
    let m = send(&test, PlacementMsg::cancel("r9").unwrap()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    ask(&test, "r3", &["gpu"]).await;
    assert_eq!(watched(&test).await, ["cicada.placement/runners"], "a refused ask records no watch");
}

/// A release and a grant from the line to one target in one turn leave it watched; a cancel of its waiting ask, while
/// it still holds, too.
#[tokio::test(start_paused = true)]
async fn a_release_and_a_line_grant_to_one_target_leave_it_watched() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask_for(&test, "a1", &["arm64"], "a").await;
    ask_for(&test, "a2", &["arm64"], "a").await;
    ask_for(&test, "a3", &["arm64"], "a").await;
    assert_eq!(
        (holders(&test).await, line(&test).await),
        (vec![holder("a1", 1, "mac-1")], vec!["a2".into(), "a3".into()])
    );
    release(&test, "a1", 1).await;
    assert_eq!(holders(&test).await, [holder("a2", 2, "mac-1")]);
    assert_eq!(watched(&test).await, ["cicada.placement/runners", "run/a"]);
    send(&test, PlacementMsg::cancel("a3").unwrap()).await;
    assert_eq!(watched(&test).await, ["cicada.placement/runners", "run/a"], "it still holds");
    release(&test, "a2", 2).await;
    assert_eq!(watched(&test).await, ["cicada.placement/runners"]);
}

/// Scenario: A run deleted after its grant. The place goes to the waiting ask.
#[tokio::test(start_paused = true)]
async fn a_holder_deleted_after_its_grant_frees_the_place_for_the_line() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    delete(&test, "r1").await;
    assert_eq!(holders(&test).await, [holder("r2", 2, "mac-1")]);
    assert!(line(&test).await.is_empty());
    assert_eq!(answers(&test, "r2").await, [QUEUED, granted(2, "mac-1")]);
    assert_eq!(watched(&test).await, ["cicada.placement/runners", "run/r2"], "the gone target is unwatched");
    assert_eq!(done(&test, "Stopped").await, 1);
}

/// Scenario: A run deleted before its grant. Deleted while it waits, or after the release that would grant it and
/// before that turn: it gets no place, and the place stays free.
#[tokio::test(start_paused = true)]
async fn a_target_deleted_before_its_grant_gets_no_place() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    ask(&test, "r3", &["arm64"]).await;
    delete(&test, "r2").await;
    assert_eq!((holders(&test).await, line(&test).await), (vec![holder("r1", 1, "mac-1")], vec!["r3".into()]));
    test.send(&pool(), PlacementMsg::release("r1", 1).unwrap()).await;
    test.system().delete_actor(&addr(&run("r3"))).await.unwrap();
    test.run_until_idle().await;
    let state = held(&test).await;
    assert!(state.holders.is_empty() && state.waiting.is_empty(), "{state:?}");
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1")]);
    assert_eq!(watched(&test).await, ["cicada.placement/runners"]);
    ask(&test, "r4", &["arm64"]).await;
    assert_eq!(answers(&test, "r4").await, [granted(3, "mac-1")], "the place stayed free");
}

/// Scenario: Two holders of one target. Both places are freed, once each, and go to the line.
#[tokio::test(start_paused = true)]
async fn a_deleted_target_with_two_holders_frees_both_places() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    ask_for(&test, "a1", &["arm64"], "a").await;
    ask_for(&test, "a2", &["arm64"], "a").await;
    ask_for(&test, "a3", &["arm64"], "a").await;
    for id in ["b1", "b2", "b3"] {
        ask(&test, id, &["arm64"]).await;
    }
    delete(&test, "a").await;
    assert_eq!(holders(&test).await, [holder("b1", 3, "mac-1"), holder("b2", 4, "mac-1")]);
    assert_eq!(line(&test).await, ["b3"], "two places freed, not more");
    assert_eq!(watched(&test).await, ["cicada.placement/runners", "run/b1", "run/b2", "run/b3"]);
}

/// Scenario: A failed message of a holder. The pool is told of it and nothing changes.
#[tokio::test(start_paused = true)]
async fn a_failed_message_of_a_holder_changes_nothing() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    let before = held(&test).await;
    let m = test.send(&run("r1"), RunMsg::Fail).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!(done(&test, "Failed").await, 1, "the pool was told");
    assert_eq!(held(&test).await, before);
    assert_eq!(watched(&test).await, ["run/r1", "run/r2"]);
}

/// A `Stopped` from a client, about a holder or the pool itself, and a real one for a target that holds nothing any
/// more change nothing.
#[tokio::test(start_paused = true)]
async fn a_stopped_for_the_pool_a_client_or_a_target_that_holds_nothing_changes_nothing() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    let before = held(&test).await;
    for actor in [addr(&run("r1")), addr(&pool())] {
        let stopped = Stopped { actor, cause: StopCause::Deleted, by: None };
        let m = send(&test, PlacementMsg::Stopped(stopped)).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    }
    assert_eq!(held(&test).await, before, "only a notification counts");
    test.send(&pool(), PlacementMsg::release("r1", 1).unwrap()).await;
    test.system().delete_actor(&addr(&run("r1"))).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(holders(&test).await, [holder("r2", 2, "mac-1")]);
    assert_eq!(watched(&test).await, ["run/r2"]);
}

/// An ask whose target is the pool itself is refused for good and records nothing.
#[tokio::test(start_paused = true)]
async fn an_ask_to_the_pool_itself_is_refused() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    let m = send(&test, PlacementMsg::ask("p1", ["arm64"], &pool(), "Placed").unwrap()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!(failed(&test).await.last(), Some(&("Ask".to_owned(), FailureCause::Refused)));
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version));
    assert!(watched(&test).await.is_empty());
}

fn gone(grant: u64, node: &str) -> (PlacementAnswerKind, Option<u64>, Option<String>) {
    (PlacementAnswerKind::Gone, Some(grant), Some(node.to_owned()))
}

/// Scenario: Draining a node with work. The ask only it could take waits, its holders keep their places, and after
/// `Undrain` the ask is granted.
#[tokio::test(start_paused = true)]
async fn a_draining_node_gets_no_new_places_and_keeps_its_holders_until_undrain() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 3).await;
    offer(&test, "x86-1", &["x86_64"], 3).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    send(&test, PlacementMsg::drain("mac-1").unwrap()).await;
    assert!(held(&test).await.nodes["mac-1"].draining);
    ask(&test, "r3", &["arm64"]).await;
    assert_eq!(answers(&test, "r3").await, [QUEUED], "queued, not refused: the never-meet check counts it");
    ask(&test, "x1", &["x86_64"]).await;
    assert_eq!(answers(&test, "x1").await, [granted(3, "x86-1")], "another node still grants");
    assert_eq!(holders(&test).await, [holder("r1", 1, "mac-1"), holder("r2", 2, "mac-1"), holder("x1", 3, "x86-1")]);
    release(&test, "r1", 1).await;
    assert_eq!(line(&test).await, ["r3"], "a freed place on a draining node grants nothing");
    send(&test, PlacementMsg::undrain("mac-1").unwrap()).await;
    assert!(!held(&test).await.nodes["mac-1"].draining);
    assert_eq!(answers(&test, "r3").await, [QUEUED, granted(4, "mac-1")], "undrain scans the line");
    assert!(failed(&test).await.is_empty());
}

/// An offer keeps the drain, also after the node went silent and came back; repeated drains and undrains change
/// nothing.
#[tokio::test(start_paused = true)]
async fn an_offer_keeps_the_drain_and_repeats_change_nothing() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    send(&test, PlacementMsg::drain("mac-1").unwrap()).await;
    let before = held(&test).await;
    let m = send(&test, PlacementMsg::drain("mac-1").unwrap()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(held(&test).await, before, "draining a draining node");
    offer(&test, "mac-1", &["arm64", "docker"], 4).await;
    assert!(held(&test).await.nodes["mac-1"].draining, "an offer keeps the drain");
    test.advance_time_by(secs(60)).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Silent);
    offer(&test, "mac-1", &["arm64"], 4).await;
    let state = held(&test).await;
    assert_eq!(status_of(&state, "mac-1"), PlacementNodeStatus::Live);
    assert!(state.nodes["mac-1"].draining, "back from silence, still draining");
    ask(&test, "r1", &["arm64"]).await;
    assert_eq!(answers(&test, "r1").await, [QUEUED]);
    send(&test, PlacementMsg::undrain("mac-1").unwrap()).await;
    let before = held(&test).await;
    let m = send(&test, PlacementMsg::undrain("mac-1").unwrap()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(held(&test).await, before, "undraining a node that is not draining");
    assert_eq!(answers(&test, "r1").await, [QUEUED, granted(1, "mac-1")]);
}

/// Scenario: An unknown node. `Drain`, `Undrain` and `Gone` of a node the pool does not know, or with a node id out of
/// bounds, are refused for good and change nothing.
#[tokio::test(start_paused = true)]
async fn drain_undrain_and_gone_of_an_unknown_node_are_refused() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    let messages = [
        PlacementMsg::drain("mac-9").unwrap(),
        PlacementMsg::undrain("mac-9").unwrap(),
        PlacementMsg::gone("mac-9").unwrap(),
        PlacementMsg::Drain { node: " ".into() },
        PlacementMsg::Undrain { node: "n".repeat(201) },
        PlacementMsg::Gone { node: String::new() },
    ];
    let count = messages.len();
    for message in messages {
        let m = send(&test, message.clone()).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed), "{message:?}");
    }
    let told = failed(&test).await;
    assert_eq!(told.len(), count, "{told:?}");
    assert!(told.iter().all(|(_, cause)| *cause == FailureCause::Refused), "{told:?}");
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version));
    assert!(PlacementMsg::drain("").is_err() && PlacementMsg::undrain("a\0").is_err());
    assert!(PlacementMsg::gone("n".repeat(201)).is_err());
}

/// Scenario: Gone for a live node. Also one whose silence ended but whose check has not run yet.
#[tokio::test(start_paused = true)]
async fn gone_of_a_live_node_is_refused_and_changes_nothing() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    let m = send(&test, PlacementMsg::gone("mac-1").unwrap()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!(failed(&test).await, [("Gone".to_owned(), FailureCause::Refused)]);
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version));
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1")]);
}

/// Scenario: A machine that will not come back. Both holders get one `gone` answer, the node is removed, a later release
/// of those grants frees nothing, waiting asks stay, and the node offering again is a new node.
#[tokio::test(start_paused = true)]
async fn gone_of_a_silent_node_frees_its_places_and_tells_each_holder_once() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64", "gpu"], 2).await;
    offer(&test, "mac-2", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;
    ask(&test, "r3", &["arm64"]).await;
    ask(&test, "g1", &["gpu"]).await;
    assert_eq!(holders(&test).await, [holder("r1", 1, "mac-1"), holder("r2", 2, "mac-1"), holder("r3", 3, "mac-2")]);
    send(&test, PlacementMsg::drain("mac-1").unwrap()).await;
    test.advance_time_by(secs(30)).await;
    offer(&test, "mac-2", &["arm64"], 1).await;
    test.advance_time_by(secs(30)).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Silent);
    ask(&test, "r4", &["arm64"]).await;
    let m = send(&test, PlacementMsg::gone("mac-1").unwrap()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1"), gone(1, "mac-1")]);
    assert_eq!(answers(&test, "r2").await, [granted(2, "mac-1"), gone(2, "mac-1")]);
    assert_eq!(answers(&test, "r3").await, [granted(3, "mac-2")], "a holder of another node hears nothing");
    let state = held(&test).await;
    assert!(!state.nodes.contains_key("mac-1"));
    assert_eq!(holders(&test).await, [holder("r3", 3, "mac-2")]);
    assert_eq!(line(&test).await, ["g1", "r4"], "waiting asks stay");
    assert_eq!(done(&test, "TellGone").await, 2);
    for grant in [1, 2] {
        let m = release(&test, &format!("r{grant}"), grant).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    }
    assert_eq!(holders(&test).await, [holder("r3", 3, "mac-2")], "a late release frees nothing");
    assert_eq!(watched(&test).await, ["run/g1", "run/r3", "run/r4"], "the told holders are unwatched");
    offer(&test, "mac-1", &["arm64"], 2).await;
    let entry = &held(&test).await.nodes["mac-1"];
    assert!(!entry.draining && entry.status == PlacementNodeStatus::Live, "a new node: {entry:?}");
    assert_eq!(answers(&test, "r4").await, [QUEUED, granted(4, "mac-1")], "the line is served by the new node");
    assert_eq!(line(&test).await, ["g1"], "no node has gpu now: it still waits");
    assert!(failed(&test).await.is_empty());
}

/// Scenario: One holder's target is gone. It is deleted after the `Gone` commits and before its `TellGone`: only its own
/// answer fails, and the other two holders are told.
#[tokio::test(start_paused = true)]
async fn a_gone_target_does_not_stop_the_other_holders_answers() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 3).await;
    for id in ["r1", "r2", "r3"] {
        ask(&test, id, &["arm64"]).await;
    }
    test.advance_time_by(secs(60)).await;
    assert_eq!(status_of(&held(&test).await, "mac-1"), PlacementNodeStatus::Silent);
    let m = test.send(&pool(), PlacementMsg::gone("mac-1").unwrap()).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    test.system().delete_actor(&addr(&run("r2"))).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1"), gone(1, "mac-1")]);
    assert_eq!(answers(&test, "r3").await, [granted(3, "mac-1"), gone(3, "mac-1")]);
    assert_eq!(failed(&test).await, [("TellGone".to_owned(), FailureCause::Commit)]);
    let state = held(&test).await;
    assert!(state.nodes.is_empty() && state.holders.is_empty(), "{state:?}");
}

/// Scenario: A TellGone from someone else. From a client or another actor it is refused for good and changes nothing.
#[tokio::test(start_paused = true)]
async fn a_tell_gone_from_another_sender_is_refused() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    let (before, version) = (held(&test).await, test.version(&pool()).await);
    let tell = PlacementMsg::TellGone {
        holder: "r1".into(),
        grant: 1,
        node: "mac-1".into(),
        to: addr(&run("r1")),
        message_type: "Placed".into(),
    };
    let m = send(&test, tell.clone()).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    test.send(&Forger, ForgerMsg::Forward { message: tell }).await;
    test.run_until_idle().await;
    let refused = ("TellGone".to_owned(), FailureCause::Refused);
    assert_eq!(failed(&test).await, [refused.clone(), refused]);
    assert_eq!((held(&test).await, test.version(&pool()).await), (before, version));
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1")]);
}

async fn offer_from(test: &TestActors, node: &str, instance: &str, labels: &[&str], places: u32) -> Message {
    send(test, PlacementMsg::offer(node, instance, labels.iter().copied(), places).unwrap()).await
}

fn conflict(instance: &str, start: Timestamp, after: u64) -> Option<PlacementConflict> {
    Some(PlacementConflict { instance: instance.into(), at: time::add(start, secs(after)).unwrap(), host: None })
}

fn told_conflict(node: &str) -> (String, PlacementNoticeKind) {
    (node.to_owned(), PlacementNoticeKind::Conflict)
}

/// Scenario: Two machines under one id. Three offers of a second instance change no entry field, the turns succeed,
/// and one `conflict` alert goes out; the first machine keeps renewing and its places are granted.
#[tokio::test(start_paused = true)]
async fn a_second_instance_under_a_live_node_changes_nothing_and_alerts_once() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 2).await;
    let first = held(&test).await.nodes["mac-1"].clone();
    for after in [10, 20, 30] {
        test.advance_time_by(secs(10)).await;
        let m = offer_from(&test, "mac-1", "i2", &["x86_64"], 7).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "an accepted turn");
        let entry = held(&test).await.nodes["mac-1"].clone();
        assert_eq!(entry.conflict, conflict("i2", start, after), "the mark follows the last conflicting offer");
        let mut unchanged = entry.clone();
        unchanged.conflict = None;
        assert_eq!(unchanged, first, "no entry field changes");
    }
    assert_eq!(notices(&test).await, [told_conflict("mac-1")], "one alert while the mark lasts");
    test.advance_time_by(secs(5)).await;
    offer(&test, "mac-1", &["arm64"], 3).await;
    let entry = held(&test).await.nodes["mac-1"].clone();
    assert_eq!((entry.places, entry.heard_at), (3, time::add(start, secs(35)).unwrap()), "the first machine renews");
    assert_eq!(entry.conflict, conflict("i2", start, 30), "the mark stays within one silence time");
    ask(&test, "r1", &["arm64"]).await;
    assert_eq!(answers(&test, "r1").await, [granted(1, "mac-1")]);
    assert_eq!(done(&test, "Alert").await, 1);
    assert!(failed(&test).await.is_empty());
}

/// Scenario: A third instance. It updates the mark, and no new alert goes out.
#[tokio::test(start_paused = true)]
async fn a_third_instance_updates_the_mark_without_an_alert() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 2).await;
    test.advance_time_by(secs(5)).await;
    offer_from(&test, "mac-1", "i2", &["arm64"], 2).await;
    test.advance_time_by(secs(5)).await;
    offer_from(&test, "mac-1", "i3", &["arm64"], 2).await;
    let entry = held(&test).await.nodes["mac-1"].clone();
    assert_eq!((entry.instance.as_str(), entry.conflict), ("i1", conflict("i3", start, 10)));
    assert_eq!(notices(&test).await, [told_conflict("mac-1")]);
}

/// Scenario: A replaced machine. The first machine stops: the node goes silent, which clears the mark, and the second
/// machine's next offer takes over the entry and is told `back`.
#[tokio::test(start_paused = true)]
async fn after_the_first_machine_goes_silent_the_second_takes_over_and_the_mark_clears() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 2).await;
    for _ in 0..3 {
        test.advance_time_by(secs(15)).await;
        offer_from(&test, "mac-1", "i2", &["x86_64"], 1).await;
    }
    test.advance_time_by(secs(15)).await;
    let state = held(&test).await;
    assert_eq!(status_of(&state, "mac-1"), PlacementNodeStatus::Silent, "silent from the first machine's last offer");
    assert_eq!(state.nodes["mac-1"].conflict, None, "going silent clears the mark");
    offer_from(&test, "mac-1", "i2", &["x86_64"], 1).await;
    let entry = held(&test).await.nodes["mac-1"].clone();
    assert_eq!((entry.instance.as_str(), entry.labels.clone(), entry.places), ("i2", vec!["x86_64".into()], 1));
    assert_eq!((entry.status, entry.heard_at), (PlacementNodeStatus::Live, time::add(start, secs(60)).unwrap()));
    assert_eq!(entry.conflict, None);
    let kinds = [PlacementNoticeKind::Conflict, PlacementNoticeKind::Silent, PlacementNoticeKind::Back];
    assert_eq!(notices(&test).await, kinds.map(|kind| ("mac-1".to_owned(), kind)));
    assert!(failed(&test).await.is_empty());
}

/// Scenario: The second machine stops. The first keeps offering; its first offer one silence time after the last
/// conflicting offer clears the mark, and a later offer of the second machine marks the node again with a new alert.
#[tokio::test(start_paused = true)]
async fn after_the_second_machine_stops_for_one_silence_time_the_mark_clears_and_a_later_conflict_alerts_again() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 2).await;
    test.advance_time_by(secs(10)).await;
    offer_from(&test, "mac-1", "i2", &["arm64"], 2).await;
    test.advance_time_by(secs(5)).await;
    for _ in 0..3 {
        offer(&test, "mac-1", &["arm64"], 2).await;
        test.advance_time_by(secs(15)).await;
    }
    offer(&test, "mac-1", &["arm64"], 2).await;
    assert_eq!(held(&test).await.nodes["mac-1"].conflict, conflict("i2", start, 10), "50 s after the conflict: kept");
    test.advance_time_by(secs(10)).await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    let entry = held(&test).await.nodes["mac-1"].clone();
    assert_eq!((entry.status, entry.conflict), (PlacementNodeStatus::Live, None), "60 s after the conflict: cleared");
    test.advance_time_by(secs(5)).await;
    offer_from(&test, "mac-1", "i2", &["arm64"], 2).await;
    assert_eq!(held(&test).await.nodes["mac-1"].conflict, conflict("i2", start, 75));
    assert_eq!(notices(&test).await, [told_conflict("mac-1"), told_conflict("mac-1")], "a new mark, a new alert");
    assert!(failed(&test).await.is_empty());
}

async fn report(test: &TestActors, node: &str, host: &str) -> Message {
    send(test, PlacementMsg::Duplicate { node: node.into(), host: host.into() }).await
}

fn reported(instance: &str, start: Timestamp, after: u64, host: &str) -> Option<PlacementConflict> {
    let at = time::add(start, secs(after)).unwrap();
    Some(PlacementConflict { instance: instance.into(), at, host: Some(host.into()) })
}

/// Scenario: A copied identity folder. Reports mark the node with the other host and its own instance, with one alert;
/// the node keeps its holder and offers but gets no new places, and one silence time after the last report the next
/// turn ends the mark and grants the waiting ask.
#[tokio::test(start_paused = true)]
async fn a_reported_duplicate_alerts_once_gives_no_new_places_and_ends_one_silence_time_after_the_last_report() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 2).await;
    ask(&test, "r1", &["arm64"]).await;
    let m = report(&test, "mac-1", "mac-b").await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(held(&test).await.nodes["mac-1"].conflict, reported("i1", start, 0, "mac-b"));
    ask(&test, "r2", &["arm64"]).await;
    assert_eq!(answers(&test, "r2").await, [QUEUED], "no new places while marked");
    for after in [15, 30] {
        test.advance_time_by(secs(15)).await;
        offer(&test, "mac-1", &["arm64"], 2).await;
        report(&test, "mac-1", "mac-c").await;
        assert_eq!(held(&test).await.nodes["mac-1"].conflict, reported("i1", start, after, "mac-c"), "the last report");
    }
    assert_eq!(notices(&test).await, [told_conflict("mac-1")], "one alert per mark");
    assert_eq!(holders(&test).await, [holder("r1", 1, "mac-1")], "the holder stays");
    for _ in 0..3 {
        test.advance_time_by(secs(15)).await;
        offer(&test, "mac-1", &["arm64"], 2).await;
    }
    assert_eq!(held(&test).await.nodes["mac-1"].conflict, reported("i1", start, 30, "mac-c"), "45 s after: kept");
    assert_eq!(answers(&test, "r2").await, [QUEUED]);
    test.advance_time_by(secs(15)).await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    let entry = held(&test).await.nodes["mac-1"].clone();
    assert_eq!((entry.status, entry.conflict), (PlacementNodeStatus::Live, None), "60 s after the last report");
    assert_eq!(answers(&test, "r2").await, [QUEUED, granted(2, "mac-1")], "places again");
    report(&test, "mac-1", "mac-b").await;
    assert_eq!(notices(&test).await, [told_conflict("mac-1"), told_conflict("mac-1")], "a new mark, a new alert");
    assert!(failed(&test).await.is_empty());
}

/// A report on a node marked by another instance's offer keeps that instance and adds the host without a new alert;
/// such a mark ends as an offer's mark does, from the later of the two. An offer of another instance on a reported node
/// keeps the host.
#[tokio::test(start_paused = true)]
async fn a_report_and_a_conflicting_offer_share_one_mark() {
    let test = node().await;
    let start = test.now();
    offer(&test, "mac-1", &["arm64"], 2).await;
    offer_from(&test, "mac-1", "i2", &["arm64"], 2).await;
    test.advance_time_by(secs(10)).await;
    report(&test, "mac-1", "mac-b").await;
    assert_eq!(held(&test).await.nodes["mac-1"].conflict, reported("i2", start, 10, "mac-b"));
    test.advance_time_by(secs(10)).await;
    offer_from(&test, "mac-1", "i3", &["arm64"], 2).await;
    assert_eq!(held(&test).await.nodes["mac-1"].conflict, reported("i3", start, 20, "mac-b"), "the host is kept");
    assert_eq!(notices(&test).await, [told_conflict("mac-1")]);
    test.advance_time_by(secs(50)).await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    assert_eq!(held(&test).await.nodes["mac-1"].conflict, None, "60 s after the last conflict, at an offer");

    // A mark set only by reports on another node.
    offer(&test, "mac-2", &["arm64"], 1).await;
    report(&test, "mac-2", "mac-x").await;
    test.advance_time_by(secs(5)).await;
    offer_from(&test, "mac-2", "i2", &["arm64"], 1).await;
    let at = test.now();
    let conflict = held(&test).await.nodes["mac-2"].conflict.clone();
    assert_eq!(conflict, Some(PlacementConflict { instance: "i2".into(), at, host: Some("mac-x".into()) }));
    assert!(failed(&test).await.is_empty());
}

/// A report of a silent node changes nothing; one of an unknown node, or out of bounds, is refused for good.
#[tokio::test(start_paused = true)]
async fn a_report_of_a_silent_node_changes_nothing_and_of_an_unknown_node_is_refused() {
    let test = node().await;
    offer(&test, "mac-1", &["arm64"], 2).await;
    test.advance_time_by(secs(60)).await;
    let before = held(&test).await;
    assert_eq!(status_of(&before, "mac-1"), PlacementNodeStatus::Silent);
    let m = report(&test, "mac-1", "mac-b").await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!(held(&test).await, before);
    for (node, host) in [("mac-9", "mac-b"), ("mac-1", " "), (" ", "mac-b")] {
        let m = report(&test, node, host).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed), "{node:?} {host:?}");
    }
    assert_eq!(held(&test).await, before);
    assert_eq!(notices(&test).await, [("mac-1".to_owned(), PlacementNoticeKind::Silent)]);
}

/// `POST /actors/cicada.placement/runners/messages` on `api` with the type and payload that
/// `cicada send cicada.placement/runners --type {message_type} --payload '{payload}'` sends, then runs what is due: the
/// status of the answer.
async fn send_over_http(test: &TestActors, api: &axum::Router, message_type: &str, payload: Value) -> u16 {
    use tower::ServiceExt;
    let request = axum::http::Request::post("/actors/cicada.placement/runners/messages")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(json!({ "type": message_type, "payload": payload }).to_string()))
        .unwrap();
    let status = api.clone().oneshot(request).await.unwrap().status().as_u16();
    test.run_until_idle().await;
    status
}

/// `GET /actors/cicada.placement/runners` on `api`, which `cicada describe actor cicada.placement/runners -o json`
/// prints: the pool's state.
async fn state_over_http(api: &axum::Router) -> Value {
    use tower::ServiceExt;
    let request = axum::http::Request::get("/actors/cicada.placement/runners").body(axum::body::Body::empty()).unwrap();
    let response = api.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice::<Value>(&body).unwrap()["state"].clone()
}

/// The holders in a state read over HTTP, as holder, grant and node.
fn holders_in(state: &Value) -> Vec<(String, u64, String)> {
    let holders = state["holders"].as_array().unwrap();
    holders
        .iter()
        .map(|h| holder(h["holder"].as_str().unwrap(), h["grant"].as_u64().unwrap(), h["node"].as_str().unwrap()))
        .collect()
}

/// The operator's recipes: the documented `cicada send` payloads of `Drain`, `Undrain`, `Gone` and `Release`, posted to
/// the send route the command uses, act on the pool, and its state read over HTTP shows the nodes, holders and line.
#[tokio::test(start_paused = true)]
async fn the_documented_cicada_send_payloads_drain_undrain_release_and_declare_gone() {
    let test = node().await;
    let api = cicada_axum::actor_api(test.system().clone());
    offer(&test, "mac-1", &["arm64"], 2).await;
    offer(&test, "mac-2", &["arm64"], 1).await;
    ask(&test, "r1", &["arm64"]).await;
    ask(&test, "r2", &["arm64"]).await;

    assert_eq!(send_over_http(&test, &api, "Drain", json!({"node": "mac-1"})).await, 202);
    let state = state_over_http(&api).await;
    assert_eq!(state["nodes"]["mac-1"]["draining"], json!(true));
    ask(&test, "r3", &["arm64"]).await;
    ask(&test, "r4", &["arm64"]).await;
    assert_eq!(answers(&test, "r4").await, [QUEUED], "mac-2 is full and mac-1 drains");

    assert_eq!(send_over_http(&test, &api, "Release", json!({"holder": "r1", "grant": 1})).await, 202);
    let state = state_over_http(&api).await;
    assert_eq!(holders_in(&state), [holder("r2", 2, "mac-1"), holder("r3", 3, "mac-2")]);
    assert_eq!(state["waiting"][0]["holder"], json!("r4"), "a freed place on a draining node grants nothing");

    assert_eq!(send_over_http(&test, &api, "Undrain", json!({"node": "mac-1"})).await, 202);
    let state = state_over_http(&api).await;
    assert_eq!(state["nodes"]["mac-1"]["draining"], json!(false));
    assert_eq!(state["waiting"], json!([]));
    assert_eq!(answers(&test, "r4").await, [QUEUED, granted(4, "mac-1")]);

    test.advance_time_by(secs(30)).await;
    offer(&test, "mac-2", &["arm64"], 1).await;
    test.advance_time_by(secs(30)).await;
    assert_eq!(state_over_http(&api).await["nodes"]["mac-1"]["status"], json!("silent"));
    assert_eq!(send_over_http(&test, &api, "Gone", json!({"node": "mac-1"})).await, 202);
    let state = state_over_http(&api).await;
    assert!(state["nodes"].get("mac-1").is_none(), "{state}");
    assert_eq!(holders_in(&state), [holder("r3", 3, "mac-2")]);
    assert_eq!(answers(&test, "r2").await, [granted(2, "mac-1"), gone(2, "mac-1")]);
    assert_eq!(answers(&test, "r4").await, [QUEUED, granted(4, "mac-1"), gone(4, "mac-1")]);
    assert!(failed(&test).await.is_empty());
}
