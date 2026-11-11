//! A run that *ended* but overran its maximum turn in blocking code (which cancellation cannot stop) is a timeout, but
//! it is only retried when the handler itself failed: a handler that returned and a permanent failure must not be
//! re-run. A run that was cut off at its maximum turn is a timeout retried per policy.
//! Real time: blocking code does not advance tokio's paused clock.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::common::*;
use crate::ActorSystem;
use crate::plugin::{HandlerErrorContext, HookResult, Plugin};
use crate::prelude::*;
use crate::store::{Backend, Message, MessageStatus, OutcomeStatus, Store};
use crate::turn::HandlerFailure;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const MAX_TURN: Duration = Duration::from_millis(200);
const BLOCK: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Laggard {
    id: String,
}

impl ActorRef for Laggard {
    const TYPE: &'static str = "laggard";
    type Message = LaggardMsg;
}

fn slow() -> Laggard {
    Laggard { id: "s".into() }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
enum Ending {
    Return,
    Throw,
    Permanent,
    Fail,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum LaggardMsg {
    /// Blocks its thread for `millis` (ignoring cancellation), then ends as `how` says.
    Stuck { millis: u64, how: Ending },
    /// Sleeps for `millis` (cancellable), then returns.
    Doze { millis: u64 },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct LaggardState {
    #[serde(default)]
    note: String,
}

struct LaggardActor;

impl Actor for LaggardActor {
    type Ref = Laggard;
    type State = LaggardState;
    type Message = LaggardMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;
    const MAX_TURN: Option<Duration> = Some(MAX_TURN);

    async fn handle(&self, turn: &mut Turn<Self>, message: LaggardMsg) -> HandlerResult {
        match message {
            LaggardMsg::Stuck { millis, how } => {
                std::thread::sleep(Duration::from_millis(millis));
                turn.state_mut().note = "blocked".into();
                match how {
                    Ending::Return => Ok(()),
                    Ending::Throw => Err(HandlerError::msg("broke after blocking")),
                    Ending::Permanent => Err(HandlerError::permanent("declined after blocking")),
                    Ending::Fail => Err(turn.fail("vetoed after blocking")),
                }
            }
            LaggardMsg::Doze { millis } => {
                tokio::time::sleep(Duration::from_millis(millis)).await;
                turn.state_mut().note = "suspended".into();
                Ok(())
            }
        }
    }
}

#[derive(Default, Clone)]
struct Observer {
    seen: Events,
    causes: Arc<Mutex<Vec<Option<String>>>>,
}

#[async_trait]
impl Plugin for Observer {
    fn name(&self) -> &str {
        "Observer"
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
        self.seen.push(format!(
            "{}:willRetry={}:{}",
            failure_kind(ctx.error()),
            ctx.will_retry(),
            ctx.error().reason()
        ));
        let cause = match ctx.error() {
            HandlerFailure::Timeout { cause, .. } => cause.as_ref().map(|c| c.reason()),
            _ => None,
        };
        self.causes.lock().unwrap().push(cause);
        Ok(())
    }
}

fn policy() -> RetryPolicy {
    RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(60)).unwrap()).unwrap()
}

struct Scenario {
    system: ActorSystem,
    memory: Store,
    observer: Observer,
}

impl Scenario {
    async fn new() -> Self {
        let memory = Store::memory();
        let observer = Observer::default();
        let system = ActorSystem::builder(memory.clone()).host(LaggardActor).install(observer.clone()).build();
        system.create(&slow(), &LaggardState::default()).await.unwrap();
        Self { system, memory, observer }
    }

    async fn accepted(&self, message: LaggardMsg) -> Message {
        accepted(self.system.send_with(&slow(), message, SendOptions::new().retry(policy())).await.unwrap())
    }

    async fn run(&self) {
        assert_eq!(self.system.run_due_turns(1).await.unwrap(), 1);
    }

    /// The message ended failed(timeout) after one run, not retried.
    async fn assert_timed_out_for_good(&self, m: &Message) {
        let outcome = outcome_of(&self.memory, &m.id).await.expect("a final outcome, no retry");
        assert_eq!(outcome.status, OutcomeStatus::Failed);
        assert_eq!(outcome.error.as_deref(), Some("timeout"));
        assert_eq!(outcome.envelope.failures, 0);
    }

    /// The message is back in the mailbox for its next attempt after a timeout.
    async fn assert_retried_after_timeout(&self, m: &Message) {
        match self.memory.status(&m.id).await.unwrap() {
            Some(MessageStatus::Pending(pending)) => {
                assert_eq!(pending.failures, 1);
                assert_eq!(pending.last_error.as_deref(), Some("timeout"));
            }
            other => panic!("retried per policy, was {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_handler_that_returned_after_overrunning_ends_failed_timeout_and_is_not_re_run() {
    let s = Scenario::new().await;
    let m = s.accepted(LaggardMsg::Stuck { millis: BLOCK, how: Ending::Return }).await;
    s.run().await;
    s.assert_timed_out_for_good(&m).await;
    assert_eq!(s.observer.seen.all(), vec!["TurnTimeoutException:willRetry=false:timeout"]);
    assert_eq!(state_of::<LaggardState>(&s.memory, &addr(&slow())).await, LaggardState::default(), "effects discarded");
}

#[tokio::test]
async fn a_permanent_failure_after_overrunning_is_not_retried() {
    let s = Scenario::new().await;
    let thrown = s.accepted(LaggardMsg::Stuck { millis: BLOCK, how: Ending::Permanent }).await;
    s.run().await;
    s.assert_timed_out_for_good(&thrown).await;
    let vetoed = s.accepted(LaggardMsg::Stuck { millis: BLOCK, how: Ending::Fail }).await;
    s.run().await;
    s.assert_timed_out_for_good(&vetoed).await;
    assert_eq!(s.observer.seen.all(), vec!["TurnTimeoutException:willRetry=false:timeout"; 2]);
}

#[tokio::test]
async fn a_handler_that_failed_after_overrunning_is_a_timeout_retried_per_policy() {
    let s = Scenario::new().await;
    let m = s.accepted(LaggardMsg::Stuck { millis: BLOCK, how: Ending::Throw }).await;
    s.run().await;
    s.assert_retried_after_timeout(&m).await;
    assert_eq!(s.observer.seen.all(), vec!["TurnTimeoutException:willRetry=true:timeout"]);
    // The handler's own error is kept as the timeout's cause.
    assert_eq!(*s.observer.causes.lock().unwrap(), vec![Some("broke after blocking".to_owned())]);
}

#[tokio::test]
async fn a_run_cut_off_at_max_turn_is_a_timeout_retried_per_policy() {
    let s = Scenario::new().await;
    let m = s.accepted(LaggardMsg::Doze { millis: 10_000 }).await;
    s.run().await;
    s.assert_retried_after_timeout(&m).await;
    assert_eq!(s.observer.seen.all(), vec!["TurnTimeoutException:willRetry=true:timeout"]);
}
