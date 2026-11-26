//! The placement pool, on virtual time with `TestActors`: offers that renew and replace, labels sorted and each once,
//! silence with one notice and the return with `back`, the check armed at the earliest end of a silence, a lost check
//! and `Rearm`, a `Check` or `Alert` from another sender, a failing alert target, a pool without a target, and the
//! bounds of an offer and of the pool.

mod common;

use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::RetryResult;
use cicada::store::{Message, OutcomeStatus};
use cicada::time::{self, Timestamp};
use cicada::{Failed, FailureCause};
use cicada_actors::{
    Placement, PlacementActor, PlacementMsg, PlacementNodeStatus, PlacementNotice, PlacementNoticeKind, PlacementState,
};
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};

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

/// A node with the pool type, `ops/pager`, the forger and the witness, which watches the pools, and the pool `runners`
/// created with a silence of `silence` seconds, at most `max_nodes` nodes, and its notices going to `ops/pager`.
async fn node_with(silence: u64, max_nodes: u32) -> TestActors {
    let test = TestActors::builder()
        .start_at("2026-01-01T00:00:00Z".parse().unwrap())
        .with(|node| node.host(PlacementActor).host(OpsActor).host(ForgerActor).host(WitnessActor))
        .build()
        .await
        .unwrap();
    test.create(&ops(), &Notices::default()).await;
    test.create(&Forger, &NoState {}).await;
    witness(&test, "cicada.placement").await;
    test.create(&pool(), &PlacementState::new(secs(silence), max_nodes).unwrap()).await;
    send(&test, PlacementMsg::alerts(&ops(), "Placement").unwrap()).await;
    test
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
        instance: "i2".into(),
        labels: vec!["docker".into(), "arm64".into(), "docker".into()],
        places: 2,
    };
    send(&test, renewal).await;
    let state = held(&test).await;
    let entry = &state.nodes["mac-1"];
    assert_eq!(
        (entry.instance.as_str(), entry.labels.clone(), entry.places),
        ("i2", vec!["arm64".into(), "docker".into()], 2)
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
    test.create(&quiet, &PlacementState::new(secs(10), 1).unwrap()).await;
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
    };
    let invalid = [
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
