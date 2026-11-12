//! The sender's per-message retry policy, enforced by the dispatcher, in virtual time.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::common::*;
use crate::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
use crate::prelude::*;
use crate::runtime::{NodeConfig, SendResult};
use crate::store::{Backend, Delivery, MessageStatus, NodeId, OutcomeStatus, OutgoingMessage, TurnOutcome};
use crate::testing::TestActors;
use crate::time::{self, MAX_DELAY, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chore {
    id: String,
}

impl ActorRef for Chore {
    const TYPE: &'static str = "chore";
    type Message = ChoreMsg;
}

fn chore(id: &str) -> Chore {
    Chore { id: id.into() }
}

/// Fails while its attempt (`failures + 1`) is at most `fail_until`, sleeping `sleep_seconds` on attempt 1 first.
/// Succeeds otherwise; replies `Reply` to the sender when there is one.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AttemptSpec {
    fail_until: u32,
    /// Fail with `turn.fail(reason)` instead of an ordinary error.
    #[serde(default)]
    veto_with_fail: bool,
    /// Fail with a permanent error built from another error.
    #[serde(default)]
    veto_with_subclass: bool,
    #[serde(default)]
    sleep_seconds: u64,
    /// Fail with a message containing a NUL character.
    #[serde(default)]
    nul: bool,
    /// Fail with a message of this many characters (when positive).
    #[serde(default)]
    error_length: usize,
}

fn fail_until(n: u32) -> AttemptSpec {
    AttemptSpec { fail_until: n, ..AttemptSpec::default() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ChoreMsg {
    Attempt(AttemptSpec),
    /// A reply.
    Reply {
        by: String,
    },
    /// Makes the receiver ask `worker` for `attempt`, with a correlation id, a header and `retry`.
    Request {
        worker: String,
        attempt: AttemptSpec,
        retry: Option<RetryPolicy>,
    },
    /// Makes the receiver forward `attempt` to `worker`, passing on the incoming message's own retry policy.
    Relay {
        worker: String,
        attempt: AttemptSpec,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct ChoreState {
    #[serde(default)]
    log: Vec<String>,
}

/// A permanent failure built from an error of the handler's own.
#[derive(Debug)]
struct Declined(String);

impl std::fmt::Display for Declined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Declined {}

/// Every handler run of `Attempt`: (attempt = failures + 1, store time).
type Runs = Arc<Mutex<Vec<(u32, Timestamp)>>>;

struct ChoreActor(Runs);

impl Actor for ChoreActor {
    type Ref = Chore;
    type State = ChoreState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ChoreMsg) -> HandlerResult {
        match message {
            ChoreMsg::Attempt(work) => {
                let attempt = turn.message().failures + 1;
                self.0.lock().unwrap().push((attempt, turn.now()));
                turn.state_mut().log.push(format!("ran:{attempt}"));
                turn.send_self_with(
                    ChoreMsg::Reply { by: "side effect of a failed turn must never commit".into() },
                    SendOptions::new().delay(Duration::from_secs(60)),
                )?;
                if attempt == 1 && work.sleep_seconds > 0 {
                    tokio::time::sleep(Duration::from_secs(work.sleep_seconds)).await;
                }
                if attempt <= work.fail_until {
                    if work.veto_with_fail {
                        return Err(turn.fail(format!("declined {attempt}")));
                    }
                    if work.veto_with_subclass {
                        return Err(HandlerError::permanent_from(Declined(format!("declined by subclass {attempt}"))));
                    }
                    if work.nul {
                        return Err(HandlerError::msg(format!("flaky\u{0}{attempt}")));
                    }
                    if work.error_length > 0 {
                        return Err(HandlerError::msg("e".repeat(work.error_length)));
                    }
                    return Err(HandlerError::msg(format!("flaky {attempt}")));
                }
                let last_error = opt(&turn.message().last_error);
                turn.state_mut().log.push(format!("ok:{attempt}:{last_error}"));
                if let Some(from) = turn.message().from.clone() {
                    let mut options = SendOptions::new();
                    options.correlation_id = turn.message().correlation_id.clone();
                    let by = turn.self_ref().id.clone();
                    turn.send_to(&from, &ChoreMsg::Reply { by }, options).await?;
                }
            }
            ChoreMsg::Reply { by } => {
                let corr = opt(&turn.message().correlation_id);
                turn.state_mut().log.push(format!("reply:{by}:{corr}"));
            }
            ChoreMsg::Request { worker, attempt, retry } => {
                let mut options = SendOptions::new().correlation_id("corr-1").header("trace", "t-1");
                options.retry = retry;
                turn.send_with(&chore(&worker), ChoreMsg::Attempt(attempt), options)?;
            }
            ChoreMsg::Relay { worker, attempt } => {
                let mut options = SendOptions::new();
                options.retry = turn.message().retry.clone();
                turn.send_with(&chore(&worker), ChoreMsg::Attempt(attempt), options)?;
            }
        }
        Ok(())
    }
}

fn fixed(max_attempts: u32, delay: Duration) -> RetryPolicy {
    RetryPolicy::new(max_attempts, Backoff::fixed(delay).unwrap()).unwrap()
}

fn exponential4() -> RetryPolicy {
    RetryPolicy::new(4, Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(300)).unwrap()).unwrap()
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

struct Harness {
    test: TestActors,
    runs: Runs,
}

impl Harness {
    async fn new(observers: Vec<Arc<dyn TurnObserver>>) -> Self {
        Self::configured(observers, None, None).await
    }

    async fn configured(
        observers: Vec<Arc<dyn TurnObserver>>,
        max_turn: Option<Duration>,
        start: Option<Timestamp>,
    ) -> Self {
        let runs = Runs::default();
        let actor = ChoreActor(runs.clone());
        let mut builder = TestActors::builder().with(move |n| {
            let mut n = n.host(actor);
            if let Some(default_max_turn) = max_turn {
                n = n.config(NodeConfig { default_max_turn, ..NodeConfig::default() });
            }
            for observer in observers {
                n = n.observe(observer);
            }
            n
        });
        if let Some(start) = start {
            builder = builder.start_at(start);
        }
        let test = builder.build().await.unwrap();
        test.create(&chore("j"), &ChoreState::default()).await;
        Self { test, runs }
    }

    fn attempts(&self) -> Vec<u32> {
        self.runs.lock().unwrap().iter().map(|r| r.0).collect()
    }

    fn times(&self, start: Timestamp) -> Vec<Duration> {
        self.runs.lock().unwrap().iter().map(|r| time::between(start, r.1)).collect()
    }

    async fn send(&self, work: AttemptSpec, retry: Option<RetryPolicy>) -> crate::store::Message {
        let mut options = SendOptions::new();
        options.retry = retry;
        self.test.send_with(&chore("j"), ChoreMsg::Attempt(work), options).await
    }

    async fn pending_status(&self, id: &crate::store::MessageId) -> crate::store::Message {
        match self.test.store().status(id).await.unwrap() {
            Some(MessageStatus::Pending(m)) => m,
            other => panic!("not pending: {other:?}"),
        }
    }

    async fn failure_ids(&self) -> Vec<crate::store::MessageId> {
        self.test.failures().await.into_iter().map(|f| f.message_id).collect()
    }
}

#[tokio::test(start_paused = true)]
async fn without_a_policy_a_failed_run_ends_the_message_failed_at_once() {
    let h = Harness::new(vec![]).await;
    let original = h.send(fail_until(1), None).await;
    h.test.advance_time_by(secs(60)).await;
    assert_eq!(h.attempts(), vec![1]);
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert_eq!(record.error.as_deref(), Some("flaky 1"));
    assert_eq!(record.envelope.failures, 0);
    assert!(record.envelope.retry.is_none());
    assert_eq!(h.failure_ids().await, vec![original.id]);
}

#[tokio::test(start_paused = true)]
async fn retries_with_exponential_backoff_and_ends_failed_after_max_attempts() {
    let h = Harness::new(vec![]).await;
    let start = h.test.now();
    let original = h.send(fail_until(99), Some(exponential4())).await;
    assert_eq!(original.retry, Some(exponential4()));
    h.test.advance_time_by(secs(60)).await;
    assert_eq!(h.attempts(), vec![1, 2, 3, 4]);
    assert_eq!(h.times(start), vec![secs(0), secs(1), secs(3), secs(7)]);

    // The same message ran four times and ended failed once, with the last handler error.
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert_eq!(record.error.as_deref(), Some("flaky 4"));
    assert_eq!(record.envelope.id, original.id);
    assert_eq!(record.envelope.attempt, 4);
    assert_eq!(record.envelope.failures, 3);
    assert_eq!(record.envelope.last_error.as_deref(), Some("flaky 3"));
    assert_eq!(record.envelope.retry, Some(exponential4()), "the policy is retained with the outcome");
    assert_eq!(h.failure_ids().await, vec![original.id]);
    // Nothing the failed handler buffered was ever committed.
    assert_eq!(h.test.state::<_, ChoreState>(&chore("j")).await, ChoreState::default());
    assert!(h.test.pending(&chore("j")).await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn between_attempts_the_message_is_pending_with_its_failures_and_policy_and_a_later_success_ends_it_done() {
    let h = Harness::new(vec![]).await;
    let policy = RetryPolicy::new(5, Backoff::exponential(secs(1), 2.0, secs(60)).unwrap()).unwrap();
    let original = h.send(fail_until(2), Some(policy)).await;
    h.test.run_until_idle().await;

    let first = h.pending_status(&original.id).await;
    assert_eq!(first.failures, 1);
    assert_eq!(first.last_error.as_deref(), Some("flaky 1"));
    assert_eq!(first.retry, original.retry);
    assert_eq!(time::between(h.test.now(), first.deliver_at), secs(1));
    assert!(first.seq > original.seq, "queued again");
    assert_eq!(
        h.test.pending(&chore("j")).await.into_iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![original.id.clone()]
    );
    assert!(h.test.outcome(&original).await.is_none());
    assert!(h.test.failures().await.is_empty(), "a retried failure is not a failure");

    h.test.advance_time_by(secs(1)).await;
    let second = h.pending_status(&original.id).await;
    assert_eq!(second.failures, 2);
    assert_eq!(second.last_error.as_deref(), Some("flaky 2"));
    assert_eq!(time::between(h.test.now(), second.deliver_at), secs(2));

    h.test.advance_time_by(secs(2)).await;
    assert_eq!(h.attempts(), vec![1, 2, 3]);
    assert_eq!(h.test.state::<_, ChoreState>(&chore("j")).await.log, vec!["ran:3", "ok:3:flaky 2"]);
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Done);
    assert!(record.error.is_none());
    assert_eq!(record.envelope.failures, 2);
    assert!(h.test.failures().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn permanent_failures_end_the_message_failed_at_once_whatever_the_policy() {
    let h = Harness::new(vec![]).await;
    let via_fail = h.send(AttemptSpec { veto_with_fail: true, ..fail_until(99) }, Some(exponential4())).await;
    let via_subclass = h.send(AttemptSpec { veto_with_subclass: true, ..fail_until(99) }, Some(exponential4())).await;
    h.test.advance_time_by(secs(60)).await;
    assert_eq!(h.attempts(), vec![1, 1]);
    let record = h.test.outcome(&via_fail).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert_eq!(record.error.as_deref(), Some("declined 1"), "the reason given to fail()");
    assert_eq!(record.envelope.failures, 0);
    let record = h.test.outcome(&via_subclass).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert_eq!(record.error.as_deref(), Some("declined by subclass 1"));
    assert_eq!(record.envelope.failures, 0);
    assert_eq!(
        h.test.state::<_, ChoreState>(&chore("j")).await,
        ChoreState::default(),
        "effects discarded as for any failure"
    );
}

/// Fails transiently on the first run and permanently afterwards.
struct TurningPermanent(Runs);

impl Actor for TurningPermanent {
    type Ref = Chore;
    type State = ChoreState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ChoreMsg) -> HandlerResult {
        if let ChoreMsg::Attempt(_) = message {
            self.0.lock().unwrap().push((turn.message().failures + 1, turn.now()));
            return Err(if turn.message().failures == 0 {
                HandlerError::msg("transient")
            } else {
                turn.fail("now permanent")
            });
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_permanent_failure_on_a_later_attempt_stops_the_retries() {
    let runs = Runs::default();
    let actor = TurningPermanent(runs.clone());
    let test = TestActors::builder().with(move |n| n.host(actor)).build().await.unwrap();
    test.create(&chore("j"), &ChoreState::default()).await;
    let original =
        test.send_with(&chore("j"), ChoreMsg::Attempt(fail_until(99)), SendOptions::new().retry(exponential4())).await;
    test.advance_time_by(secs(60)).await;
    assert_eq!(runs.lock().unwrap().iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 2]);
    let record = test.outcome(&original).await.unwrap();
    assert_eq!(record.error.as_deref(), Some("now permanent"));
    assert_eq!(record.envelope.failures, 1);
    assert_eq!(record.envelope.last_error.as_deref(), Some("transient"));
}

#[tokio::test(start_paused = true)]
async fn crash_recovery_between_retries_does_not_use_up_an_attempt() {
    let h = Harness::new(vec![]).await;
    let original = h.send(fail_until(99), Some(fixed(2, secs(5)))).await;
    h.test.run_until_idle().await; // attempt 1 fails: retried
    tokio::time::sleep(secs(5)).await; // virtual time only; no turn runs
    // Another node claims the message and dies without committing.
    let types: BTreeSet<_> = [type_name::<Chore>()].into();
    let lost = h.test.store().claim_due(&types, &NodeId::new("crashed").unwrap(), secs(30), 1).await.unwrap();
    assert_eq!(lost.len(), 1);
    let lost = &lost[0];
    assert_eq!(lost.message.id, original.id);
    assert_eq!(lost.message.attempt, 2);
    assert_eq!(lost.message.failures, 1);
    assert_eq!(lost.message.retry, original.retry);

    h.test.advance_time_by(secs(60)).await; // the lease expires and this node recovers the turn
    assert_eq!(h.attempts(), vec![1, 2], "two handler runs: the lost claim was not a failure");
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert_eq!(record.error.as_deref(), Some("flaky 2"));
    assert_eq!(record.envelope.attempt, 3, "attempt counts claims, including the lost one");
    assert_eq!(record.envelope.failures, 1);
}

#[tokio::test(start_paused = true)]
async fn a_turn_timeout_is_retried_like_a_failure() {
    let h = Harness::configured(vec![], Some(secs(10)), None).await;
    let start = h.test.now();
    // Attempt 1 times out, attempt 2 is quick.
    let original = h.send(AttemptSpec { sleep_seconds: 60, ..fail_until(0) }, Some(fixed(2, secs(5)))).await;
    h.test.advance_time_by(secs(60)).await;
    assert_eq!(h.attempts(), vec![1, 2]);
    assert_eq!(h.times(start), vec![secs(0), secs(15)]);
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Done);
    assert_eq!(record.envelope.failures, 1);
    assert_eq!(record.envelope.last_error.as_deref(), Some("timeout"));
    assert_eq!(h.test.state::<_, ChoreState>(&chore("j")).await.log, vec!["ran:2", "ok:2:timeout"]);
}

#[tokio::test(start_paused = true)]
async fn a_retry_is_the_same_message_so_replies_still_reach_the_requester() {
    let h = Harness::new(vec![]).await;
    let test = &h.test;
    test.create(&chore("requester"), &ChoreState::default()).await;
    test.create(&chore("worker"), &ChoreState::default()).await;
    let policy = fixed(3, secs(2));
    test.send(
        &chore("requester"),
        ChoreMsg::Request { worker: "worker".into(), attempt: fail_until(1), retry: Some(policy.clone()) },
    )
    .await;
    test.run_until_idle().await;
    let asked = test.pending(&chore("worker")).await.remove(0);
    assert_eq!(asked.failures, 1);
    assert_eq!(asked.from, Some(addr(&chore("requester"))));
    assert_eq!(asked.correlation_id.as_deref(), Some("corr-1"));
    assert_eq!(asked.headers, [("trace".to_owned(), "t-1".to_owned())].into());
    assert_eq!(asked.retry, Some(policy), "a handler's send carries the policy it was given");

    test.advance_time_by(secs(10)).await;
    assert_eq!(test.state::<_, ChoreState>(&chore("worker")).await.log, vec!["ran:2", "ok:2:flaky 1"]);
    assert_eq!(test.state::<_, ChoreState>(&chore("requester")).await.log, vec!["reply:worker:corr-1"]);
    assert_eq!(test.outcome(&asked).await.unwrap().status, OutcomeStatus::Done);
}

#[tokio::test(start_paused = true)]
async fn a_handler_forwards_the_incoming_policy_with_message_retry() {
    let h = Harness::new(vec![]).await;
    let test = &h.test;
    test.create(&chore("relay"), &ChoreState::default()).await;
    test.create(&chore("worker"), &ChoreState::default()).await;
    let policy = fixed(2, secs(1));
    test.send_with(
        &chore("relay"),
        ChoreMsg::Relay { worker: "worker".into(), attempt: fail_until(1) },
        SendOptions::new().retry(policy.clone()),
    )
    .await;
    test.run_until_idle().await;
    let forwarded = test.pending(&chore("worker")).await.remove(0);
    assert_eq!(forwarded.retry, Some(policy));
    test.advance_time_by(secs(5)).await;
    let record = test.outcome(&forwarded).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Done);
    assert_eq!(record.envelope.failures, 1);
}

/// An observer that writes the run and the outcome of every end: `<run>:<outcome>`, see [`outcome_text`].
fn decision_observer(seen: &Events) -> Arc<dyn TurnObserver> {
    let seen = seen.clone();
    Arc::new(OnEnd::new(move |message, end| {
        seen.push(format!("{}:{}", message.failures + 1, outcome_text(end.outcome())));
    }))
}

#[tokio::test(start_paused = true)]
async fn observers_see_the_runtimes_decision_for_every_failed_run() {
    let seen = Events::default();
    let h = Harness::new(vec![decision_observer(&seen)]).await;
    let original = h.send(fail_until(99), Some(fixed(3, secs(1)))).await;
    let unretried = h.send(fail_until(1), None).await;
    h.test.advance_time_by(secs(60)).await;
    assert_eq!(
        seen.all(),
        vec![
            "1:Rescheduled(delivery=After(delay=1s), error=flaky 1)",
            "1:Failed(error=flaky 1)",
            "2:Rescheduled(delivery=After(delay=1s), error=flaky 2)",
            "3:Failed(error=flaky 3)",
        ]
    );
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    assert_eq!(record.envelope.failures, 2);
    assert_eq!(h.test.outcome(&unretried).await.unwrap().status, OutcomeStatus::Failed);
    // An observer only observes: nothing but the retries was committed, and handler effects never.
    assert_eq!(h.test.state::<_, ChoreState>(&chore("j")).await, ChoreState::default());
    assert!(h.test.pending(&chore("j")).await.is_empty());
}

/// Panics at the start of every turn and at its end.
struct Broken;

impl TurnObserver for Broken {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        if turn.message().failures == 0 {
            panic!("observer bug at the start");
        }
        Box::new(Broken)
    }
}

impl ObservedTurn for Broken {
    fn turn_ended(self: Box<Self>, _end: &TurnEnd<'_>) {
        panic!("observer bug at the end")
    }
}

#[tokio::test(start_paused = true)]
async fn a_panicking_observer_is_ignored_and_the_runtimes_decision_stands() {
    let h = Harness::new(vec![Arc::new(Broken)]).await;
    let unretried = h.send(fail_until(1), None).await;
    let retried = h.send(fail_until(1), Some(fixed(2, secs(1)))).await;
    h.test.advance_time_by(secs(60)).await;
    let record = h.test.outcome(&unretried).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed, "no retry without a policy");
    assert_eq!(record.envelope.failures, 0);
    let record = h.test.outcome(&retried).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Done, "the retry stood");
    assert_eq!(record.envelope.failures, 1);
}

/// An observer that writes the run, the kind of the outcome and its error of every end.
fn outcome_observer(seen: &Events) -> Arc<dyn TurnObserver> {
    let seen = seen.clone();
    Arc::new(OnEnd::new(move |message, end| {
        let kind = match end.outcome().status() {
            None => "rescheduled",
            Some(status) => status.wire_name(),
        };
        seen.push(format!("{}:{kind}:{}", message.failures + 1, opt(&end.outcome().error())));
    }))
}

#[tokio::test(start_paused = true)]
async fn an_observer_sees_each_retried_run_and_the_final_outcome() {
    let seen = Events::default();
    let h = Harness::new(vec![outcome_observer(&seen)]).await;
    h.send(fail_until(99), Some(fixed(3, secs(1)))).await;
    h.test.advance_time_by(secs(60)).await;
    assert_eq!(seen.all(), vec!["1:rescheduled:flaky 1", "2:rescheduled:flaky 2", "3:failed:flaky 3"]);
}

#[tokio::test(start_paused = true)]
async fn max_attempts_1_never_retries() {
    let h = Harness::new(vec![]).await;
    let original = h.send(fail_until(99), Some(fixed(1, secs(1)))).await;
    h.test.advance_time_by(secs(60)).await;
    assert_eq!(h.attempts().len(), 1);
    assert_eq!(h.test.outcome(&original).await.unwrap().status, OutcomeStatus::Failed);
}

#[tokio::test(start_paused = true)]
async fn load_failures_and_undecodable_messages_are_not_retried() {
    let h = Harness::new(vec![]).await;
    let test = &h.test;
    // Undecodable payload, as accepted by a non-hosting node.
    let mut bad = OutgoingMessage::new(addr(&chore("j")), "Attempt", json!({"failUntil": "x"}));
    bad.retry = Some(exponential4());
    let bad = enqueue(test.store(), bad).await;
    // Undecodable state.
    test.store().create(&addr(&chore("broken")), json!({"log": 1})).await.unwrap();
    let to_broken = test
        .send_with(&chore("broken"), ChoreMsg::Attempt(fail_until(0)), SendOptions::new().retry(exponential4()))
        .await;
    test.advance_time_by(secs(60)).await;
    for m in [&bad, &to_broken] {
        let record = test.outcome(m).await.unwrap();
        assert_eq!(record.status, OutcomeStatus::Failed);
        assert_eq!(record.envelope.failures, 0);
    }
    assert_eq!(test.failures().await.len(), 2);
    assert!(h.attempts().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_duplicate_send_with_a_different_policy_is_accepted_and_ignored() {
    let h = Harness::new(vec![]).await;
    let first = h
        .test
        .send_with(
            &chore("j"),
            ChoreMsg::Attempt(fail_until(1)),
            SendOptions::new().delay(secs(60)).retry(exponential4()),
        )
        .await;
    let again = h
        .test
        .system()
        .send_with(
            &chore("j"),
            ChoreMsg::Attempt(fail_until(1)),
            SendOptions::new().id(first.id.clone()).retry(fixed(1, secs(1))),
        )
        .await
        .unwrap();
    assert!(matches!(again, SendResult::Duplicate(_)), "{again:?}");
    assert_eq!(h.test.pending(&chore("j")).await[0].retry, Some(exponential4()), "the first accepted send stands");
}

fn year_9500() -> Timestamp {
    time::parse_instant("9500-01-01T00:00:00Z").unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_retry_the_store_cannot_schedule_ends_failed_with_the_handlers_error() {
    // Near MAX_INSTANT the delay is beyond it, and the store refuses the reschedule.
    let h = Harness::configured(vec![], None, Some(year_9500())).await;
    let original = h.send(fail_until(99), Some(fixed(5, MAX_DELAY))).await;
    h.test.run_until_idle().await;
    assert_eq!(h.attempts(), vec![1]);
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    let error = record.error.unwrap();
    assert!(error.starts_with("flaky 1"), "{error}");
    assert!(error.contains("(commit refused: "), "{error}");
    assert_eq!(record.envelope.failures, 0);
    assert_eq!(h.failure_ids().await, vec![original.id]);
}

#[tokio::test(start_paused = true)]
async fn a_long_failure_is_recorded_as_last_error_truncated_to_max_error_length() {
    let h = Harness::new(vec![]).await;
    let original = h.send(AttemptSpec { error_length: 5_000, ..fail_until(1) }, Some(fixed(2, secs(1)))).await;
    h.test.run_until_idle().await;
    let pending = h.pending_status(&original.id).await;
    let last_error = pending.last_error.unwrap();
    assert_eq!(last_error.chars().count(), TurnOutcome::MAX_ERROR_LENGTH);
    assert!(last_error.starts_with("eee"));
}

#[tokio::test(start_paused = true)]
async fn a_failure_whose_text_contains_a_nul_character_is_still_retried_with_the_character_replaced() {
    let h = Harness::new(vec![]).await;
    let original = h.send(AttemptSpec { nul: true, ..fail_until(1) }, Some(fixed(2, Duration::from_millis(1)))).await;
    h.test.advance_time_by(secs(60)).await;
    assert_eq!(h.attempts(), vec![1, 2]);
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Done);
    assert_eq!(record.envelope.last_error.as_deref(), Some("flaky\u{FFFD}1"));
}

/// An observer that keeps the outcome of every end.
fn outcome_recorder(seen: &Arc<Mutex<Vec<TurnOutcome>>>) -> Arc<dyn TurnObserver> {
    let seen = seen.clone();
    Arc::new(OnEnd::new(move |_, end| seen.lock().unwrap().push(end.outcome().clone())))
}

#[tokio::test(start_paused = true)]
async fn a_refused_retry_leaves_state_and_mailbox_untouched_and_the_observer_sees_the_handlers_failure() {
    // Near MAX_INSTANT the store refuses the reschedule, so the reason says "commit refused". Nothing of the refused
    // commit is applied.
    let seen = Arc::new(Mutex::new(Vec::new()));
    let h = Harness::configured(vec![outcome_recorder(&seen)], None, Some(year_9500())).await;
    let original = h.send(fail_until(99), Some(fixed(5, MAX_DELAY))).await;
    h.test.run_until_idle().await;
    assert_eq!(h.attempts(), vec![1]);
    let record = h.test.outcome(&original).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed);
    let error = record.error.clone().unwrap();
    assert!(error.starts_with("flaky 1 (commit refused: "), "{error}");
    assert_eq!(record.envelope.failures, 0, "not rescheduled");
    assert_eq!(h.test.state::<_, ChoreState>(&chore("j")).await, ChoreState::default(), "state untouched");
    assert_eq!(h.test.version(&chore("j")).await, 0);
    assert!(h.test.pending(&chore("j")).await.is_empty(), "no retry");
    assert_eq!(*seen.lock().unwrap(), vec![TurnOutcome::Failed(error)]);
}

#[tokio::test(start_paused = true)]
async fn an_observer_sees_the_truncated_last_error_of_a_retry() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let h = Harness::new(vec![outcome_recorder(&seen)]).await;
    h.send(AttemptSpec { error_length: 5_000, ..fail_until(1) }, Some(fixed(2, secs(1)))).await;
    h.test.run_until_idle().await;
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    match &seen[0] {
        TurnOutcome::Rescheduled { delivery, error } => {
            assert_eq!(error.chars().count(), TurnOutcome::MAX_ERROR_LENGTH);
            assert_eq!(*delivery, Delivery::After(secs(1)));
        }
        other => panic!("not rescheduled: {other:?}"),
    }
}
