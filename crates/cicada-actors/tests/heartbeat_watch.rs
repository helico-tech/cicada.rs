//! The heartbeat watch, on virtual time with `TestActors`: waiting until a first `Beat`, a synthetic first beat with
//! the create, beats that replace the timer, one alarm per silence, recovery by a later `Beat`, early, forged, stale,
//! retried and redriven timers, `Rearm` after a lost or failed timer, both orders of a `Beat` and a timer due together,
//! pauses, restarts and deletes, fractional and overflowing deadlines, the checks of the target, the envelope of the
//! alarm, observer headers, payload depth and a lost claim.

mod common;

use std::time::Duration;

use cicada::observer::{ObservedTurn, Sends, TurnObserver, TurnStart};
use cicada::prelude::*;
use cicada::runtime::{NodeConfig, RetryResult};
use cicada::store::{Message, OutcomeStatus};
use cicada::time::{self, MAX_INSTANT, Timestamp};
use cicada::{Failed, FailureCause, OpError, WatchTarget};
use cicada_actors::{
    DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, HeartbeatStatus, HeartbeatWatch,
    HeartbeatWatchActor, HeartbeatWatchMsg, HeartbeatWatchState, StoredMessage,
};
use cicada_test::{TestActors, claim_and_abandon};
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SECOND: Duration = Duration::from_secs(1);

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn hb() -> HeartbeatWatch {
    HeartbeatWatch::named("import")
}

/// A watch that sends `worker/<to>` a `Note` "silent" after `silence` seconds without a `Beat`.
fn settings_for(to: &str, silence: u64) -> HeartbeatWatchState {
    HeartbeatWatchState::new(secs(silence), &worker(to), WorkerMsg::Note { text: "silent".into() }).unwrap()
}

/// A watch that alarms `worker/a`, which is paused, so that its alarms wait in its mailbox.
fn settings(silence: u64) -> HeartbeatWatchState {
    settings_for("a", silence)
}

fn two_attempts() -> RetryPolicy {
    RetryPolicy::new(2, Backoff::fixed(secs(10)).unwrap()).unwrap()
}

/// The crate's shared retry policy, which the timer carries.
fn shared_policy() -> RetryPolicy {
    RetryPolicy::new(7, Backoff::exponential(SECOND, 2.0, secs(32)).unwrap()).unwrap()
}

/// A message of a family that the worker does not take.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum Shout {
    Shout { text: String },
}

/// `producer/_`: creates the watch with the settings it is given and beats it in one turn, or sends it an `Expire`
/// of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Producer;

impl ActorRef for Producer {
    const TYPE: &'static str = "producer";
    type Message = ProducerMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
enum ProducerMsg {
    Go { settings: HeartbeatWatchState },
    Forge,
}

struct ProducerActor;

impl Actor for ProducerActor {
    type Ref = Producer;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProducerMsg) -> HandlerResult {
        match message {
            ProducerMsg::Go { settings } => {
                turn.create(&hb(), &settings)?;
                turn.send(&hb(), HeartbeatWatchMsg::Beat)?;
            }
            ProducerMsg::Forge => {
                turn.send(&hb(), HeartbeatWatchMsg::Expire)?;
            }
        }
        Ok(())
    }
}

/// A node at `start` with the watch type, the workers `worker/a` (paused) and `worker/b`, the producer, the dead
/// letters (not created) and the witness, which watches the heartbeat watches.
async fn node_with(start: &str, config: Option<NodeConfig>) -> TestActors {
    let test = TestActors::builder()
        .start_at(ts(start))
        .with(move |node| {
            let node = node
                .host(HeartbeatWatchActor)
                .host(WorkerActor)
                .host(WitnessActor)
                .host(ProducerActor)
                .host(DeadLettersActor::default());
            match config {
                Some(config) => node.config(config),
                None => node,
            }
        })
        .build()
        .await
        .unwrap();
    for id in ["a", "b"] {
        test.create(&worker(id), &WorkerState::default()).await;
    }
    test.system().pause_mailbox(&addr(&worker("a"))).await.unwrap();
    test.create(&Producer, &NoState {}).await;
    witness(&test, "cicada.heartbeat-watch").await;
    test
}

async fn node_at(start: &str) -> TestActors {
    node_with(start, None).await
}

async fn node() -> TestActors {
    node_at("2026-01-01T00:00:00Z").await
}

/// Sends the watch `message` from a client and runs what is due.
async fn send(test: &TestActors, message: HeartbeatWatchMsg) -> Message {
    let m = test.send(&hb(), message).await;
    test.run_until_idle().await;
    m
}

async fn held(test: &TestActors) -> HeartbeatWatchState {
    test.state(&hb()).await
}

/// The alarms that wait for `worker/a`, as the offsets of their due times from `start`.
async fn alarms(test: &TestActors, start: Timestamp) -> Vec<Duration> {
    test.pending(&worker("a")).await.iter().map(|m| time::between(start, m.deliver_at)).collect()
}

/// The timers waiting in the mailbox of the watch, as offsets from `start`; asserts that the last is the armed one.
async fn timers(test: &TestActors, start: Timestamp) -> Vec<Duration> {
    let pending: Vec<Message> = test.pending(&hb()).await.into_iter().filter(|m| m.message_type == "Expire").collect();
    if let Some(last) = pending.last() {
        assert_eq!(held(test).await.armed.as_ref(), Some(&last.id), "the armed timer");
    }
    pending.iter().map(|m| time::between(start, m.deliver_at)).collect()
}

/// The one timer waiting in the mailbox of the watch, which is the armed one.
async fn armed(test: &TestActors) -> Message {
    let pending: Vec<Message> = test.pending(&hb()).await.into_iter().filter(|m| m.message_type == "Expire").collect();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(held(test).await.armed.as_ref(), Some(&pending[0].id), "the pending timer is the armed one");
    pending.into_iter().next().unwrap()
}

/// The `Failed` the witness was told of the watch.
async fn failed(test: &TestActors) -> Vec<Failed> {
    told(test).await.into_iter().filter(|f| f.actor == addr(&hb())).collect()
}

async fn log(test: &TestActors, id: &str) -> Vec<String> {
    test.state::<_, WorkerState>(&worker(id)).await.log
}

/// The state of a watch that `settings` describe once its round has started at `at`, with `armed` as its timer.
fn alive(settings: &HeartbeatWatchState, at: Timestamp, armed: &str) -> HeartbeatWatchState {
    let mut state = settings.clone();
    state.status = HeartbeatStatus::Alive;
    state.last_beat_at = Some(at);
    state.armed = Some(MessageId::new(armed).unwrap());
    state
}

// Starting.

/// Scenario: Start without an external first beat; a created watch waits.
#[tokio::test(start_paused = true)]
async fn a_created_watch_waits_and_arms_nothing_until_its_first_beat() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    test.advance_time_by(secs(100)).await;
    assert_eq!(held(&test).await, settings(10), "still waiting");
    assert!(test.pending(&hb()).await.is_empty() && alarms(&test, start).await.is_empty());
    send(&test, HeartbeatWatchMsg::Beat).await;
    let state = held(&test).await;
    assert_eq!((state.status, state.last_beat_at), (HeartbeatStatus::Alive, time::add(start, secs(100)).ok()));
    assert_eq!(timers(&test, start).await, [secs(110)]);
    assert!(failed(&test).await.is_empty());
}

/// Scenario: Start without an external first beat, by create plus send in one turn; a repeated create changes nothing.
#[tokio::test(start_paused = true)]
async fn a_producer_creates_a_watch_and_beats_it_in_one_turn_and_a_repeated_create_changes_nothing() {
    let test = node().await;
    let start = test.now();
    test.send(&Producer, ProducerMsg::Go { settings: settings(10) }).await;
    test.run_until_idle().await;
    assert_eq!(held(&test).await.status, HeartbeatStatus::Alive, "alive before any outside activity");
    assert_eq!(timers(&test, start).await, [secs(10)]);
    let (before, version) = (held(&test).await, test.version(&hb()).await);
    test.create(&hb(), &settings(1)).await;
    assert_eq!((held(&test).await, test.version(&hb()).await), (before, version), "settings and progress stay");
    test.advance_time_by(secs(5)).await;
    test.send(&Producer, ProducerMsg::Go { settings: settings(1) }).await;
    test.run_until_idle().await;
    let state = held(&test).await;
    assert_eq!((state.silence, state.last_beat_at), (secs(10), time::add(start, secs(5)).ok()), "only the Beat counts");
    assert_eq!(timers(&test, start).await, [secs(15)]);
}

// A Beat starts or replaces one silence round atomically.

/// Scenario: Continuing activity.
#[tokio::test(start_paused = true)]
async fn beats_within_the_silence_send_no_alarm_and_each_replaces_the_timer() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    let mut previous: Option<Message> = None;
    for n in 0..5 {
        send(&test, HeartbeatWatchMsg::Beat).await;
        assert_eq!(timers(&test, start).await, [secs(5 * n + 10)], "one armed timer");
        if let Some(old) = previous {
            assert_eq!(status(&test, &old).await, Some(OutcomeStatus::Cancelled), "the timer before it");
        }
        previous = Some(armed(&test).await);
        test.advance_time_by(secs(5)).await;
    }
    assert!(alarms(&test, start).await.is_empty());
    assert_eq!(held(&test).await.last_beat_at, time::add(start, secs(20)).ok());
}

/// Scenario: Silence produces one alarm.
#[tokio::test(start_paused = true)]
async fn a_silence_sends_one_alarm_and_arms_nothing_more() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let timer = armed(&test).await;
    test.advance_time_by(secs(9) + Duration::from_millis(999)).await;
    assert!(alarms(&test, start).await.is_empty(), "not before the deadline");
    test.advance_time_by(Duration::from_millis(1)).await;
    assert_eq!(alarms(&test, start).await, [secs(10)]);
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Done));
    let state = held(&test).await;
    assert_eq!((state.status, state.last_beat_at, state.armed), (HeartbeatStatus::Silent, Some(start), None));
    assert!(test.pending(&hb()).await.is_empty(), "no further timer");
    test.advance_time_by(secs(1000)).await;
    assert_eq!(alarms(&test, start).await.len(), 1, "one silence, one alarm");
    assert!(failed(&test).await.is_empty() && test.exists(&hb()).await, "it never stops itself");
}

/// Scenario: Recovery. The Beat after the silence starts a new round and tells nobody.
#[tokio::test(start_paused = true)]
async fn a_beat_after_the_alarm_starts_a_new_round_without_a_recovery_message() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    test.advance_time_by(secs(30)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let state = held(&test).await;
    assert_eq!((state.status, state.last_beat_at), (HeartbeatStatus::Alive, time::add(start, secs(30)).ok()));
    assert_eq!(timers(&test, start).await, [secs(40)]);
    assert_eq!(alarms(&test, start).await, [secs(10)], "no recovery message");
    test.advance_time_by(secs(10)).await;
    assert_eq!(alarms(&test, start).await, [secs(10), secs(40)], "the new round's silence alarms once");
}

// Only the armed self timer can expire a round.

/// Scenario: Early administrative delivery.
#[tokio::test(start_paused = true)]
async fn an_early_expire_made_due_by_an_operator_rearms_at_the_unchanged_deadline() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let timer = armed(&test).await;
    test.advance_time_by(secs(3)).await;
    test.system().deliver_now(&timer.id).await.unwrap();
    test.run_until_idle().await;
    assert!(alarms(&test, start).await.is_empty(), "no alarm before the measured silence");
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Done));
    let replacement = armed(&test).await;
    assert_ne!(replacement.id, timer.id);
    assert_eq!(timers(&test, start).await, [secs(10)], "the same deadline");
    assert_eq!(held(&test).await.last_beat_at, Some(start), "no activity was invented");
    test.advance_time_by(secs(7)).await;
    assert_eq!(alarms(&test, start).await, [secs(10)]);
}

/// Scenario: Forged, stale and copied timers: from a client, from another actor and an operator's retry of a
/// superseded timer change neither state, version nor outgoing messages, in each status.
#[tokio::test(start_paused = true)]
async fn forged_and_stale_expires_change_nothing_in_any_status() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let superseded = armed(&test).await;
    test.advance_time_by(secs(2)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    assert_eq!(status(&test, &superseded).await, Some(OutcomeStatus::Cancelled));
    let forge = async |test: &TestActors| {
        let (before, version, pending) = (held(test).await, test.version(&hb()).await, test.pending(&hb()).await);
        let client = send(test, HeartbeatWatchMsg::Expire).await;
        let other = test.send(&Producer, ProducerMsg::Forge).await;
        test.run_until_idle().await;
        let RetryResult::Accepted(copy) = test.system().retry_message(&superseded.id).await.unwrap() else {
            panic!("not retried")
        };
        test.run_until_idle().await;
        assert_eq!(status(test, &client).await, Some(OutcomeStatus::Done));
        assert_eq!(status(test, &other).await, Some(OutcomeStatus::Done));
        assert_eq!(status(test, &copy).await, Some(OutcomeStatus::Done));
        assert_eq!((held(test).await, test.version(&hb()).await), (before.clone(), version), "{:?}", before.status);
        assert_eq!(test.pending(&hb()).await, pending, "no timer armed or cancelled");
    };
    forge(&test).await;
    assert!(alarms(&test, start).await.is_empty(), "alive: no alarm");
    test.advance_time_by(secs(10)).await;
    assert_eq!(alarms(&test, start).await, [secs(12)], "the armed timer still counts");
    forge(&test).await;
    assert_eq!(alarms(&test, start).await.len(), 1, "silent: no second alarm");
    let waiting = HeartbeatWatch::named("waiting");
    test.create(&waiting, &settings(10)).await;
    let version = test.version(&waiting).await;
    let m = test.send(&waiting, HeartbeatWatchMsg::Expire).await;
    test.run_until_idle().await;
    assert_eq!((status(&test, &m).await, test.version(&waiting).await), (Some(OutcomeStatus::Done), version));
    assert!(failed(&test).await.is_empty());
}

// Rearm recovers the existing round without a new Beat.

/// Scenario: No active round.
#[tokio::test(start_paused = true)]
async fn a_rearm_in_waiting_or_silent_changes_nothing_and_repeated_rearms_send_no_second_alarm() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    let version = test.version(&hb()).await;
    let m = send(&test, HeartbeatWatchMsg::Rearm).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done));
    assert_eq!((held(&test).await, test.version(&hb()).await), (settings(10), version), "waiting");
    assert!(test.pending(&hb()).await.is_empty());
    send(&test, HeartbeatWatchMsg::Beat).await;
    test.advance_time_by(secs(10)).await;
    let (before, version) = (held(&test).await, test.version(&hb()).await);
    assert_eq!(before.status, HeartbeatStatus::Silent);
    for _ in 0..3 {
        send(&test, HeartbeatWatchMsg::Rearm).await;
    }
    test.advance_time_by(secs(100)).await;
    assert_eq!((held(&test).await, test.version(&hb()).await), (before, version), "silent");
    assert_eq!(alarms(&test, start).await, [secs(10)]);
    assert!(test.pending(&hb()).await.is_empty());
}

/// Scenario: Missing timer. A purged or cancelled timer leaves the round alive with its stale id and tells nobody; a
/// Rearm before the deadline arms the old deadline, one after it is due at once.
#[tokio::test(start_paused = true)]
async fn a_lost_timer_leaves_the_watch_alive_and_a_rearm_arms_the_old_deadline() {
    for (purge, wait) in [(false, 5), (true, 5), (false, 20), (true, 20)] {
        let test = node().await;
        let start = test.now();
        test.create(&hb(), &settings(10)).await;
        send(&test, HeartbeatWatchMsg::Beat).await;
        let timer = armed(&test).await;
        if purge {
            test.system().purge_mailbox(&addr(&hb())).await.unwrap();
        } else {
            test.system().cancel(&timer.id).await.unwrap();
        }
        test.advance_time_by(secs(wait)).await;
        let state = held(&test).await;
        assert_eq!((state.status, state.armed), (HeartbeatStatus::Alive, Some(timer.id.clone())), "purge {purge}");
        assert!(alarms(&test, start).await.is_empty() && failed(&test).await.is_empty(), "nobody is told");
        send(&test, HeartbeatWatchMsg::Rearm).await;
        let state = held(&test).await;
        assert_eq!(state.last_beat_at, Some(start), "no Beat was invented");
        if wait < 10 {
            assert_eq!(timers(&test, start).await, [secs(10)], "the old deadline, purge {purge}");
            test.advance_time_by(secs(10 - wait)).await;
        }
        assert_eq!(alarms(&test, start).await, [secs(10.max(wait))], "purge {purge}, wait {wait}");
        assert_eq!(held(&test).await.status, HeartbeatStatus::Silent);
    }
}

/// Scenario: Recovering a failed timer: a timer that failed for good, because its target was gone, keeps the round
/// alive; an operator's retry and a redrive of it by the dead letters are copies under new ids and change nothing; a
/// Rearm once the target is back sends the alarm.
#[tokio::test(start_paused = true)]
async fn retries_and_redrives_of_a_failed_timer_are_ignored_and_a_rearm_recovers_the_round() {
    let test = node().await;
    test.create(&DeadLetters::default(), &DeadLettersState::default()).await;
    let watched = WatchTarget::parse("cicada.heartbeat-watch/import").unwrap();
    test.system().watch(&watched, &addr(&DeadLetters::default())).await.unwrap();
    test.create(&hb(), &settings_for("c", 10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let timer = armed(&test).await;
    test.advance_time_by(secs(10)).await;
    let failure = failed(&test).await.pop().expect("the failed timer");
    assert_eq!((failure.cause, failure.message_id.clone()), (FailureCause::Commit, timer.id.clone()));
    let before = held(&test).await;
    assert_eq!((before.status, before.armed.clone()), (HeartbeatStatus::Alive, Some(timer.id.clone())));
    let version = test.version(&hb()).await;
    test.create(&worker("c"), &WorkerState::default()).await;
    let RetryResult::Accepted(retried) = test.system().retry_message(&timer.id).await.unwrap() else {
        panic!("not retried")
    };
    assert_ne!(retried.id, timer.id);
    test.run_until_idle().await;
    assert_eq!(status(&test, &retried).await, Some(OutcomeStatus::Done));
    test.send(&DeadLetters::default(), DeadLettersMsg::Redrive { message_id: timer.id.clone() }).await;
    test.run_until_idle().await;
    let letters: DeadLettersState = test.state(&DeadLetters::default()).await;
    assert!(letters.letters[&timer.id].redriven_as.is_some(), "redriven");
    assert_eq!((held(&test).await, test.version(&hb()).await), (before, version), "the copies are ignored");
    assert!(log(&test, "c").await.is_empty() && test.pending(&hb()).await.is_empty());
    send(&test, HeartbeatWatchMsg::Rearm).await;
    assert_eq!(log(&test, "c").await, ["silent"], "the overdue round alarms at once");
    assert_eq!(held(&test).await.status, HeartbeatStatus::Silent);
}

// Timing is based on handling and mailbox order.

/// Scenario: Fractional duration and bounds: a silence with a fraction below a microsecond is rounded up by the core.
#[tokio::test(start_paused = true)]
async fn a_fractional_silence_is_kept_and_its_timer_is_rounded_up_never_earlier() {
    let test = node().await;
    let start = test.now();
    let mut state = json!({"silence": "PT1.0000005S", "target": "worker/a"});
    state["alarm"] = serde_json::to_value(settings(1).alarm).unwrap();
    test.create(&hb(), &state).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    assert_eq!(held(&test).await.silence, Duration::new(1, 500));
    let timer = armed(&test).await;
    assert_eq!(timer.deliver_at, time::add(start, Duration::new(1, 1_000)).unwrap(), "the next microsecond");
    test.advance_time_by(secs(2)).await;
    let alarm = test.pending(&worker("a")).await.pop().expect("the alarm");
    assert!(alarm.deliver_at >= time::add(start, Duration::new(1, 500)).unwrap());
}

/// Scenario: Fractional duration and bounds: a Beat whose deadline would be after `MAX_INSTANT` is refused for good
/// and keeps the round and the timer before it; on a waiting watch it commits nothing either.
#[tokio::test(start_paused = true)]
async fn a_beat_whose_deadline_would_pass_max_instant_is_refused_and_changes_nothing() {
    let test = node_at("9999-12-31T23:59:54.500Z").await;
    test.create(&hb(), &settings(5)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let before = held(&test).await;
    assert_eq!(armed(&test).await.deliver_at, MAX_INSTANT.min(time::add(test.now(), secs(5)).unwrap()));
    test.advance_time_by(Duration::from_millis(500)).await;
    let m = send(&test, HeartbeatWatchMsg::Beat).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    let refused = failed(&test).await;
    assert_eq!(refused.iter().map(|f| f.cause).collect::<Vec<_>>(), [FailureCause::Refused]);
    assert!(refused[0].error.contains("after 9999-12-31"), "{}", refused[0].error);
    assert_eq!(held(&test).await, before);
    assert_eq!(armed(&test).await.id, before.armed.clone().unwrap(), "the old timer");
    let late = HeartbeatWatch::named("late");
    test.create(&late, &settings(5)).await;
    let m = test.send(&late, HeartbeatWatchMsg::Beat).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!(test.state::<_, HeartbeatWatchState>(&late).await, settings(5));
    assert!(test.pending(&late).await.is_empty());
}

/// Scenario: Equal delivery timestamps, the Beat first: sent before the first Beat is handled, its sequence is lower
/// than that of the timer the first Beat arms.
#[tokio::test(start_paused = true)]
async fn a_beat_due_with_the_timer_and_ordered_first_replaces_it_with_no_alarm() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(2)).await;
    test.send(&hb(), HeartbeatWatchMsg::Beat).await;
    test.send_with(&hb(), HeartbeatWatchMsg::Beat, SendOptions::new().delay(secs(2))).await;
    test.run_until_idle().await;
    let timer = armed(&test).await;
    test.advance_time_by(secs(2)).await;
    assert!(alarms(&test, start).await.is_empty());
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Cancelled));
    assert_eq!(timers(&test, start).await, [secs(4)]);
}

/// Scenario: Equal delivery timestamps, the timer first: it alarms, then the Beat starts a new round.
#[tokio::test(start_paused = true)]
async fn a_beat_due_with_the_timer_and_ordered_after_it_follows_the_alarm_and_starts_anew() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(2)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    test.send_with(&hb(), HeartbeatWatchMsg::Beat, SendOptions::new().delay(secs(2))).await;
    test.advance_time_by(secs(2)).await;
    assert_eq!(alarms(&test, start).await, [secs(2)]);
    let state = held(&test).await;
    assert_eq!((state.status, state.last_beat_at), (HeartbeatStatus::Alive, time::add(start, secs(2)).ok()));
    assert_eq!(timers(&test, start).await, [secs(4)]);
}

/// Scenario: Pause, slow handling and restart: a Beat ordered before the timer of a paused watch replaces it from the
/// time it is handled.
#[tokio::test(start_paused = true)]
async fn a_paused_watch_that_resumes_with_a_beat_ordered_first_counts_from_its_claim_time() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    test.system().pause_mailbox(&addr(&hb())).await.unwrap();
    test.advance_time_by(secs(5)).await;
    test.send(&hb(), HeartbeatWatchMsg::Beat).await;
    test.advance_time_by(secs(25)).await;
    test.system().resume_mailbox(&addr(&hb())).await.unwrap();
    test.run_until_idle().await;
    assert!(alarms(&test, start).await.is_empty());
    assert_eq!(held(&test).await.last_beat_at, time::add(start, secs(30)).ok(), "the claim time, not the send time");
    assert_eq!(timers(&test, start).await, [secs(40)]);
}

/// Scenario: Pause, slow handling and restart: a Beat accepted after the deadline while the watch was paused follows
/// the alarm.
#[tokio::test(start_paused = true)]
async fn a_beat_accepted_after_the_deadline_while_paused_follows_the_late_alarm() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    test.system().pause_mailbox(&addr(&hb())).await.unwrap();
    test.advance_time_by(secs(15)).await;
    test.send(&hb(), HeartbeatWatchMsg::Beat).await;
    test.advance_time_by(secs(15)).await;
    test.system().resume_mailbox(&addr(&hb())).await.unwrap();
    test.run_until_idle().await;
    assert_eq!(alarms(&test, start).await, [secs(30)], "late, when claimed");
    let state = held(&test).await;
    assert_eq!((state.status, state.last_beat_at), (HeartbeatStatus::Alive, time::add(start, secs(30)).ok()));
    assert_eq!(timers(&test, start).await, [secs(40)]);
}

/// Scenario: Pause, slow handling and restart: a node that starts later on the same store sends the alarm; a deleted
/// watch sends nothing.
#[tokio::test(start_paused = true)]
async fn a_restarted_node_keeps_the_round_and_a_deleted_watch_sends_nothing() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let later = ActorSystem::builder(test.store().clone()).host(HeartbeatWatchActor).host(WorkerActor).build();
    later.initialize().await.unwrap();
    tokio::time::sleep(secs(10)).await;
    assert_eq!(later.run_due_turns(10).await.unwrap(), 1, "the timer, on the later node");
    assert_eq!(alarms(&test, start).await, [secs(10)]);
    assert_eq!(held(&test).await.status, HeartbeatStatus::Silent);

    let gone = HeartbeatWatch::named("gone");
    test.create(&gone, &settings(10)).await;
    test.send(&gone, HeartbeatWatchMsg::Beat).await;
    test.run_until_idle().await;
    test.system().delete_actor(&addr(&gone)).await.unwrap();
    test.advance_time_by(secs(100)).await;
    assert!(!test.exists(&gone).await && test.pending(&gone).await.is_empty());
    assert_eq!(alarms(&test, start).await.len(), 1, "nothing from the deleted watch");
}

// Alarm effects and failures follow ordinary turn semantics.

/// Scenario: Self target and no-op priority.
#[tokio::test(start_paused = true)]
async fn a_watch_whose_target_is_itself_refuses_beat_and_rearm_and_keeps_its_no_ops() {
    let test = node().await;
    let own = HeartbeatWatchState::new(secs(10), &hb(), HeartbeatWatchMsg::Beat).unwrap();
    test.create(&hb(), &own).await;
    for message in [HeartbeatWatchMsg::Rearm, HeartbeatWatchMsg::Expire] {
        let m = send(&test, message).await;
        assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Done), "a no-op in waiting");
    }
    let m = send(&test, HeartbeatWatchMsg::Beat).await;
    assert_eq!(status(&test, &m).await, Some(OutcomeStatus::Failed));
    assert_eq!((held(&test).await, test.pending(&hb()).await), (own.clone(), vec![]), "nothing committed");

    let host = HeartbeatWatch::named("host");
    let own = HeartbeatWatchState::new(secs(10), &host, HeartbeatWatchMsg::Beat).unwrap();
    let supplied = alive(&own, test.now(), "e1");
    test.create(&host, &supplied).await;
    let forged = test.send(&host, HeartbeatWatchMsg::Expire).await;
    let rearm = test.send(&host, HeartbeatWatchMsg::Rearm).await;
    test.run_until_idle().await;
    assert_eq!(status(&test, &forged).await, Some(OutcomeStatus::Done), "a forged timer stays a no-op");
    assert_eq!(status(&test, &rearm).await, Some(OutcomeStatus::Failed));
    assert_eq!(test.state::<_, HeartbeatWatchState>(&host).await, supplied);
    assert!(test.pending(&host).await.is_empty());
    let refused: Vec<_> = told(&test).await.into_iter().map(|f| (f.message_type, f.cause)).collect();
    assert_eq!(refused, [("Beat".to_owned(), FailureCause::Refused), ("Rearm".to_owned(), FailureCause::Refused)]);
    assert!(told(&test).await.iter().all(|f| f.error.contains("to itself")));
}

/// Scenario: Target checks and retry policies: an unregistered type or a message the target does not take refuses the
/// timer for good, and an absent target rejects its commit; each keeps the round alive with its armed id.
#[tokio::test(start_paused = true)]
async fn an_alarm_the_target_cannot_take_is_refused_and_an_absent_target_fails_the_commit() {
    let test = node().await;
    let nobody = ActorAddress::parse("nobody/x").unwrap();
    let absent = addr(&worker("c"));
    let shout = StoredMessage::of(&Shout::Shout { text: "hey".into() }).unwrap();
    let note = settings(10).alarm;
    let cases = [
        (nobody, note.clone(), FailureCause::Refused),
        (addr(&worker("a")), shout, FailureCause::Refused),
        (absent, note, FailureCause::Commit),
    ];
    for (n, (target, alarm, cause)) in cases.into_iter().enumerate() {
        let watch = HeartbeatWatch::named(format!("w{n}"));
        let mut state = settings(10);
        (state.target, state.alarm) = (target, alarm);
        test.create(&watch, &state).await;
        test.send(&watch, HeartbeatWatchMsg::Beat).await;
        test.run_until_idle().await;
        let before: HeartbeatWatchState = test.state(&watch).await;
        test.advance_time_by(secs(10)).await;
        let failure = told(&test).await.into_iter().find(|f| f.actor == addr(&watch)).expect("the failed timer");
        assert_eq!((failure.message_type.as_str(), failure.cause), ("Expire", cause), "{watch:?}");
        assert_eq!(test.state::<_, HeartbeatWatchState>(&watch).await, before, "nothing committed: {watch:?}");
    }
    assert!(test.pending(&worker("a")).await.is_empty());
}

/// Scenario: Target checks and retry policies: a payload the target cannot decode fails there, later; the watch is
/// silent.
#[tokio::test(start_paused = true)]
async fn a_payload_the_target_cannot_decode_fails_there_and_the_watch_is_silent() {
    let test = node().await;
    let mut state = settings_for("b", 10);
    state.alarm = StoredMessage { message_type: "Note".into(), payload: json!({"text": 7}) };
    test.create(&hb(), &state).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    test.advance_time_by(secs(10)).await;
    let failures = test.failures().await;
    assert!(failures.iter().any(|f| f.envelope.message_type == "Note"), "{failures:?}");
    assert!(failed(&test).await.is_empty(), "the timer itself is done");
    assert_eq!(held(&test).await.status, HeartbeatStatus::Silent);
}

/// Scenario: Target checks and retry policies: the alarm carries the stored retry policy, and a retry at the target
/// does not pass the watch again.
#[tokio::test(start_paused = true)]
async fn the_alarm_carries_the_stored_retry_and_a_retry_at_the_target_does_not_reenter_the_watch() {
    let test = node().await;
    let mut state =
        HeartbeatWatchState::new(secs(10), &worker("b"), WorkerMsg::FailOnce { text: "again".into() }).unwrap();
    state.retry = Some(two_attempts());
    test.create(&hb(), &state).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    test.advance_time_by(secs(10)).await;
    assert!(log(&test, "b").await.is_empty(), "the first run failed");
    let version = test.version(&hb()).await;
    test.advance_time_by(secs(10)).await;
    assert_eq!(log(&test, "b").await, ["again"]);
    assert_eq!((held(&test).await.status, test.version(&hb()).await), (HeartbeatStatus::Silent, version));
    assert!(test.pending(&hb()).await.is_empty());
}

/// Scenario: Metadata and observation: the alarm has the watch as sender, a new id, the stored retry policy and no
/// correlation id or header from the Beat; the timer has the shared policy.
#[tokio::test(start_paused = true)]
async fn the_alarm_has_the_watch_as_sender_a_new_id_and_no_correlation_and_the_timer_the_shared_policy() {
    let test = node().await;
    let mut state = settings(10);
    state.retry = Some(two_attempts());
    test.create(&hb(), &state).await;
    let beat =
        test.send_with(&hb(), HeartbeatWatchMsg::Beat, SendOptions::new().correlation_id("c").header("h", "v")).await;
    test.run_until_idle().await;
    let timer = armed(&test).await;
    assert_eq!((timer.retry, timer.from.clone()), (Some(shared_policy()), Some(addr(&hb()))));
    test.advance_time_by(secs(10)).await;
    let pending = test.pending(&worker("a")).await;
    assert_eq!(pending.len(), 1);
    let alarm = &pending[0];
    assert_eq!((alarm.message_type.as_str(), &alarm.payload), ("Note", &json!({"text": "silent"})));
    assert_eq!(alarm.from, Some(addr(&hb())));
    assert!(alarm.id != timer.id && alarm.id != beat.id);
    assert_eq!((alarm.correlation_id.clone(), alarm.retry.clone()), (None, Some(two_attempts())));
    assert!(alarm.headers.is_empty(), "{:?}", alarm.headers);
}

/// Scenario: Metadata and observation: an observer's headers go on the alarm as on any send, and on the timer by the
/// selection of timers.
#[tokio::test(start_paused = true)]
async fn observer_headers_go_on_the_alarm_and_the_timer_by_their_selection() {
    struct Picks;
    impl TurnObserver for Picks {
        fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
            turn.add_header_for(Sends::Timers, "timer", "t");
            turn.add_header_for(Sends::Others, "other", "o");
            Box::new(())
        }
    }
    let test = TestActors::builder()
        .with(|node| node.host(HeartbeatWatchActor).host(WorkerActor).observe(Picks))
        .build()
        .await
        .unwrap();
    test.create(&worker("a"), &WorkerState::default()).await;
    test.system().pause_mailbox(&addr(&worker("a"))).await.unwrap();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let timer = armed(&test).await;
    assert_eq!(timer.headers, [("timer".to_owned(), "t".to_owned())].into());
    test.advance_time_by(secs(10)).await;
    let alarm = test.pending(&worker("a")).await.pop().expect("the alarm");
    assert_eq!(alarm.headers, [("other".to_owned(), "o".to_owned())].into());
}

/// A value `levels` deep: a chain of arrays around a number, built without recursion.
fn nested(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, _| Value::Array(vec![inner]))
}

/// Scenario: State depth: an alarm payload of 62 levels is a state of 64, which the create takes and the alarm sends;
/// one of 63 is refused by the create, and so is a NUL in a string.
#[tokio::test(start_paused = true)]
async fn the_create_takes_an_alarm_of_62_levels_and_refuses_63_and_a_nul() {
    let test = node().await;
    let mut state = settings(10);
    state.alarm = StoredMessage { message_type: "Note".into(), payload: nested(62) };
    test.system().create(&hb(), &state).await.unwrap();
    send(&test, HeartbeatWatchMsg::Beat).await;
    test.advance_time_by(secs(10)).await;
    assert_eq!(test.pending(&worker("a")).await.pop().expect("the alarm").payload, nested(62));
    state.alarm.payload = nested(63);
    let refused = test.system().create(&HeartbeatWatch::named("deeper"), &state).await;
    assert!(matches!(&refused, Err(OpError::Invalid(why)) if why.contains("64")), "{refused:?}");
    let nul = HeartbeatWatchState::new(secs(10), &worker("a"), WorkerMsg::Note { text: "a\0b".into() }).unwrap();
    let refused = test.system().create(&HeartbeatWatch::named("nul"), &nul).await;
    assert!(matches!(&refused, Err(OpError::Invalid(_))), "{refused:?}");
}

/// Scenario: Failure and uncertain commit: a claim of the timer that is abandoned is recovered by the core under the
/// same id and sender, which still counts, and sends the alarm once.
#[tokio::test(start_paused = true)]
async fn a_timer_whose_claim_was_abandoned_is_recovered_and_alarms_once() {
    let test = node().await;
    let start = test.now();
    test.create(&hb(), &settings(10)).await;
    send(&test, HeartbeatWatchMsg::Beat).await;
    let timer = armed(&test).await;
    tokio::time::sleep(secs(10)).await;
    let lease = secs(30);
    assert_eq!(claim_and_abandon(test.store(), &types::<HeartbeatWatch>(), lease, 1).await.unwrap().len(), 1);
    assert!(alarms(&test, start).await.is_empty(), "nothing committed");
    tokio::time::sleep(lease + SECOND).await;
    test.run_until_idle().await;
    assert_eq!(alarms(&test, start).await.len(), 1);
    assert_eq!(status(&test, &timer).await, Some(OutcomeStatus::Done));
    assert_eq!(held(&test).await.status, HeartbeatStatus::Silent);
}
