//! The exact semantics of the three hooks, including the edge cases: timeouts, undecodable payloads and state, failing
//! hooks and retries.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cicada::plugin::{ClaimContext, HandlerErrorContext, HookResult, OutcomeContext, Plugin};
use cicada::prelude::*;
use cicada::runtime::NodeConfig;
use cicada::store::{OutcomeStatus, OutgoingMessage, Store};
use cicada::time;
use cicada::turn::HandlerFailure;
use cicada_test::TestActors;
use common::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Probe {
    id: String,
}

impl ActorRef for Probe {
    const TYPE: &'static str = "probe";
    type Message = ProbeMsg;
}

fn p(id: &str) -> Probe {
    Probe { id: id.into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum ProbeMsg {
    /// Records itself, buffers a state change and a self-send, then optionally sleeps and/or fails.
    Do {
        #[serde(default)]
        fail: bool,
        #[serde(default, rename = "sleepSeconds")]
        sleep_seconds: u64,
    },
    Log {
        text: String,
    },
    /// Makes the receiver send a failing `Do` to `target`, with a correlation id, a header and the kick retry.
    Kick {
        target: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct ProbeState {
    #[serde(default)]
    log: Vec<String>,
}

/// The attribute the recorder sets in `on_claim`.
#[derive(Debug, Clone)]
struct Stamp(String);

fn kick_retry() -> RetryPolicy {
    RetryPolicy::new(2, Backoff::fixed(Duration::from_secs(5)).unwrap()).unwrap()
}

async fn handle_probe<A>(events: &Events, turn: &mut Turn<A>, message: ProbeMsg) -> HandlerResult
where
    A: Actor<Ref = Probe, State = ProbeState, Message = ProbeMsg>,
{
    match message {
        ProbeMsg::Do { fail, sleep_seconds } => {
            let m = turn.message();
            events.push(format!(
                "handler from={} corr={} x={}",
                opt(&m.from),
                opt(&m.correlation_id),
                opt(&m.headers.get("x"))
            ));
            turn.state_mut().log.push("do".into());
            turn.send_self(ProbeMsg::Log { text: "from handler".into() })?;
            if sleep_seconds > 0 {
                tokio::time::sleep(Duration::from_secs(sleep_seconds)).await;
            }
            if fail {
                return Err(HandlerError::msg("boom"));
            }
        }
        ProbeMsg::Log { text } => turn.state_mut().log.push(text),
        ProbeMsg::Kick { target } => {
            let options = SendOptions::new().correlation_id("c-1").header("x", "1").retry(kick_retry());
            turn.send_with(&p(&target), ProbeMsg::Do { fail: true, sleep_seconds: 0 }, options)?;
        }
    }
    Ok(())
}

/// The probe with the unhandled policy `fail`; its maximum turn is the node's default.
struct ProbeActor(Events);

impl Actor for ProbeActor {
    type Ref = Probe;
    type State = ProbeState;
    type Message = ProbeMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProbeMsg) -> HandlerResult {
        handle_probe(&self.0, turn, message).await
    }
}

/// Records every hook it sees.
struct Recorder(Events);

#[async_trait]
impl Plugin for Recorder {
    fn name(&self) -> &str {
        "Recorder"
    }

    async fn on_claim(&self, ctx: &ClaimContext<'_>) -> HookResult {
        let info = ctx.info();
        self.0.push(format!("claim:{}:unhandled={}", info.message().message_type, info.unhandled().is_some()));
        info.attributes().insert(Stamp(format!("s-{}", info.message().id)));
        Ok(())
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
        self.0.push(format!(
            "error:{}:{}:stamp={}",
            failure_kind(ctx.error()),
            decision_text(ctx),
            ctx.info().attributes().get::<Stamp>().is_some()
        ));
        Ok(())
    }

    async fn on_outcome(&self, ctx: &OutcomeContext<'_>) -> HookResult {
        self.0.push(format!(
            "outcome:{}:{}:sent={}:stamp={}",
            ctx.info().message().message_type,
            outcome_text(ctx.outcome()),
            ctx.sent().len(),
            ctx.info().attributes().get::<Stamp>().is_some()
        ));
        Ok(())
    }
}

/// The runtime's decision as an error hook sees it, in the text of the outcome it leads to (see [`outcome_text`]).
fn decision_text(ctx: &HandlerErrorContext<'_>) -> String {
    let error = ctx.error().reason();
    assert_eq!(ctx.will_retry(), ctx.retry_delay().is_some());
    match ctx.retry_delay() {
        Some(delay) => format!("Rescheduled(delivery=After(delay={delay:?}), error={error})"),
        None => format!("Failed(error={error})"),
    }
}

/// A probe type with a maximum turn of `max_turn` (the node's default) and `plugins`, hosted by a test harness.
async fn probe_test<A: Actor>(actor: A, max_turn: Duration, plugins: Vec<Arc<dyn Plugin>>) -> TestActors {
    TestActors::builder()
        .with(move |n| {
            let mut n = n.host(actor).config(NodeConfig { default_max_turn: max_turn, ..NodeConfig::default() });
            for plugin in plugins {
                n = n.install(plugin);
            }
            n
        })
        .build()
        .await
        .unwrap()
}

const MINUTE: Duration = Duration::from_secs(60);

#[tokio::test(start_paused = true)]
async fn a_turn_timeout_fires_on_handler_error_with_a_timeout_and_ends_failed_timeout_by_default() {
    let events = Events::default();
    let test =
        probe_test(ProbeActor(events.clone()), Duration::from_secs(30), vec![Arc::new(Recorder(events.clone()))]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 60 }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().error.as_deref(), Some("timeout"));
    assert_eq!(test.state::<_, ProbeState>(&p("p")).await, ProbeState::default()); // handler effects discarded
    assert!(test.pending(&p("p")).await.is_empty());
    assert!(
        events.all().contains(&"error:TurnTimeoutException:Failed(error=timeout):stamp=true".to_owned()),
        "{:?}",
        events.all()
    );
    assert_eq!(events.last(), "outcome:Do:Failed(error=timeout):sent=0:stamp=true");
}

struct Catcher(Arc<Mutex<Option<Duration>>>);

#[async_trait]
impl Plugin for Catcher {
    fn name(&self) -> &str {
        "Catcher"
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
        if let HandlerFailure::Timeout { max_turn, .. } = ctx.error() {
            *self.0.lock().unwrap() = Some(*max_turn);
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn an_error_hook_sees_the_exceeded_max_turn_of_a_timeout() {
    let seen = Arc::new(Mutex::new(None));
    let test =
        probe_test(ProbeActor(Events::default()), Duration::from_secs(30), vec![Arc::new(Catcher(seen.clone()))]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 60 }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().error.as_deref(), Some("timeout"));
    assert_eq!(*seen.lock().unwrap(), Some(Duration::from_secs(30)));
}

#[tokio::test(start_paused = true)]
async fn an_undecodable_payload_with_policy_fail_on_claim_sees_it_as_unhandled_no_error_hook() {
    let events = Events::default();
    let reason = Arc::new(Mutex::new(None));
    let test = probe_test(
        ProbeActor(events.clone()),
        MINUTE,
        vec![Arc::new(Recorder(events.clone())), Arc::new(ReasonReader(reason.clone()))],
    )
    .await;
    test.create(&p("p"), &ProbeState::default()).await;
    let bad =
        enqueue(&**test.store(), OutgoingMessage::new(addr(&p("p")), "Do", json!({"fail": "not a boolean"}))).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&bad).await.unwrap().status, OutcomeStatus::Failed);
    let all = events.all();
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0], "claim:Do:unhandled=true");
    assert!(all[1].starts_with("outcome:Do:Failed(error=Payload of 'Do' cannot be decoded"), "{}", all[1]);

    events.clear();
    enqueue(&**test.store(), OutgoingMessage::new(addr(&p("p")), "Throw", json!({}))).await;
    test.run_until_idle().await;
    assert_eq!(reason.lock().unwrap().as_deref(), Some("Unknown message type 'Throw'"));
    assert_eq!(
        events.all(),
        vec![
            "claim:Throw:unhandled=true",
            "outcome:Throw:Failed(error=Unknown message type 'Throw'):sent=0:stamp=true",
        ]
    );
}

struct ReasonReader(Arc<Mutex<Option<String>>>);

#[async_trait]
impl Plugin for ReasonReader {
    fn name(&self) -> &str {
        "Reason"
    }

    async fn on_claim(&self, ctx: &ClaimContext<'_>) -> HookResult {
        *self.0.lock().unwrap() = ctx.info().unhandled().map(str::to_owned);
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn state_that_cannot_be_decoded_is_a_load_failure_only_on_outcome_fires() {
    let events = Events::default();
    let test = probe_test(ProbeActor(events.clone()), MINUTE, vec![Arc::new(Recorder(events.clone()))]).await;
    test.store().create(&addr(&p("p")), json!({"log": "not a list"})).await.unwrap();
    let m = test.send(&p("p"), ProbeMsg::Do { fail: true, sleep_seconds: 0 }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Failed);
    let all = events.all();
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(all[0].starts_with("outcome:Do:Failed(error=State cannot be decoded"), "{}", all[0]);
    assert!(all[0].ends_with("stamp=false"));
}

struct Broken;

#[async_trait]
impl Plugin for Broken {
    fn name(&self) -> &str {
        "Broken"
    }

    async fn on_handler_error(&self, _ctx: &HandlerErrorContext<'_>) -> HookResult {
        anyhow::bail!("hook broke")
    }
}

struct Later(Arc<Mutex<Option<String>>>);

#[async_trait]
impl Plugin for Later {
    fn name(&self) -> &str {
        "Later"
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
        *self.0.lock().unwrap() = Some(decision_text(ctx));
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_failing_on_handler_error_hook_is_ignored_and_later_hooks_still_run() {
    let seen = Arc::new(Mutex::new(None));
    let test =
        probe_test(ProbeActor(Events::default()), MINUTE, vec![Arc::new(Broken), Arc::new(Later(seen.clone()))]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let m = test.send(&p("p"), ProbeMsg::Do { fail: true, sleep_seconds: 0 }).await;
    test.run_until_idle().await;
    assert_eq!(seen.lock().unwrap().as_deref(), Some("Failed(error=boom)"));
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Failed);
    assert_eq!(test.state::<_, ProbeState>(&p("p")).await, ProbeState::default());
    assert!(test.pending(&p("p")).await.is_empty());
}

struct Reader(Events);

impl Reader {
    fn stamp(info: &cicada::plugin::TurnInfo) -> String {
        info.attributes().get::<Stamp>().map(|s| s.0).unwrap_or_default()
    }
}

#[async_trait]
impl Plugin for Reader {
    fn name(&self) -> &str {
        "Reader"
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
        self.0.push(format!("error:{}", Self::stamp(ctx.info())));
        Ok(())
    }

    async fn on_outcome(&self, ctx: &OutcomeContext<'_>) -> HookResult {
        self.0.push(format!("outcome:{}", Self::stamp(ctx.info())));
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn attributes_set_in_on_claim_reach_the_handler_every_later_hook_and_on_outcome() {
    let seen = Events::default();
    let test = probe_test(
        ProbeActor(Events::default()),
        MINUTE,
        vec![Arc::new(Recorder(Events::default())), Arc::new(Reader(seen.clone()))],
    )
    .await;
    test.create(&p("p"), &ProbeState::default()).await;
    let failing = test.send(&p("p"), ProbeMsg::Do { fail: true, sleep_seconds: 0 }).await;
    test.run_until_idle().await;
    assert_eq!(seen.all(), vec![format!("error:s-{}", failing.id), format!("outcome:s-{}", failing.id)]);
    seen.clear();
    let ok = test.send(&p("p"), ProbeMsg::Log { text: "x".into() }).await;
    test.run_until_idle().await;
    assert_eq!(seen.all(), vec![format!("outcome:s-{}", ok.id)]);
}

/// Records every failure that is retried: the attempt and the backoff.
struct LogRetry(Events);

#[async_trait]
impl Plugin for LogRetry {
    fn name(&self) -> &str {
        "LogRetry"
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
        if ctx.will_retry() {
            self.0.push(format!(
                "hook {} after {}",
                ctx.info().message().attempt,
                opt(&ctx.retry_delay().map(|d| format!("{d:?}")))
            ));
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_retry_puts_the_same_message_back_in_place_and_the_error_hook_sees_the_retry() {
    let events = Events::default();
    let retries = Events::default();
    let test = probe_test(
        ProbeActor(events.clone()),
        MINUTE,
        vec![Arc::new(LogRetry(retries.clone())), Arc::new(Recorder(events.clone()))],
    )
    .await;
    test.create(&p("a"), &ProbeState::default()).await;
    test.create(&p("b"), &ProbeState::default()).await;
    test.send(&p("a"), ProbeMsg::Kick { target: "b".into() }).await; // a failing Do with the kick retry: 2 attempts, 5 s apart
    test.run_until_idle().await;
    let pending = test.pending(&p("b")).await;
    assert_eq!(pending.len(), 1);
    let again = pending[0].clone();
    assert_eq!(again.message_type, "Do");
    assert_eq!(again.from, Some(addr(&p("a"))));
    assert_eq!(again.correlation_id.as_deref(), Some("c-1"));
    assert_eq!(again.headers, [("x".to_owned(), "1".to_owned())].into());
    assert_eq!(again.retry, Some(kick_retry()));
    assert_eq!(again.attempt, 1);
    assert_eq!(again.failures, 1);
    assert_eq!(again.last_error.as_deref(), Some("boom"));
    assert_eq!(time::between(test.now(), again.deliver_at), Duration::from_secs(5));
    assert!(test.outcome(&again).await.is_none(), "no outcome while waiting for the retry");
    // The handler's effects are discarded; the hook saw the retry and its backoff.
    assert_eq!(test.state::<_, ProbeState>(&p("b")).await, ProbeState::default());
    assert_eq!(retries.all(), vec!["hook 1 after 5s"]);
    let all = events.all();
    assert!(
        all.contains(&"error:Error:Rescheduled(delivery=After(delay=5s), error=boom):stamp=true".to_owned()),
        "{all:?}"
    );
    assert!(
        all.contains(&"outcome:Do:Rescheduled(delivery=After(delay=5s), error=boom):sent=0:stamp=true".to_owned()),
        "{all:?}"
    );

    test.advance_time_by(Duration::from_secs(5)).await;
    let handlers: Vec<String> = events.all().into_iter().filter(|e| e.starts_with("handler")).collect();
    assert_eq!(handlers, vec!["handler from=probe/a corr=c-1 x=1"; 2]);
    let record = test.outcome(&again).await.unwrap();
    assert_eq!(record.status, OutcomeStatus::Failed, "the second failure is the last attempt: exactly one outcome");
    assert_eq!(record.error.as_deref(), Some("boom"));
    assert_eq!(record.envelope.id, again.id);
    assert_eq!(record.envelope.attempt, 2);
    assert_eq!(record.envelope.failures, 1);
    assert!(test.pending(&p("b")).await.is_empty());
    assert!(events.all().contains(&"error:Error:Failed(error=boom):stamp=true".to_owned()));
    assert_eq!(retries.all(), vec!["hook 1 after 5s"], "the last failure is not retried");
}

#[tokio::test(start_paused = true)]
async fn a_timeout_is_retried_under_the_messages_policy_and_records_timeout_as_its_error() {
    let events = Events::default();
    let test =
        probe_test(ProbeActor(events.clone()), Duration::from_secs(30), vec![Arc::new(Recorder(events.clone()))]).await;
    test.create(&p("p"), &ProbeState::default()).await;
    let retry = RetryPolicy::new(2, Backoff::fixed(Duration::from_millis(1)).unwrap()).unwrap();
    let m =
        test.send_with(&p("p"), ProbeMsg::Do { fail: false, sleep_seconds: 60 }, SendOptions::new().retry(retry)).await;
    test.advance_time_by(Duration::from_secs(300)).await;
    let record = test.outcome(&m).await.unwrap();
    assert_eq!(record.error.as_deref(), Some("timeout"));
    assert_eq!(record.envelope.attempt, 2);
    assert_eq!(record.envelope.failures, 1);
    assert_eq!(record.envelope.last_error.as_deref(), Some("timeout"));
    assert_eq!(
        time::between(record.envelope.deliver_at, record.finished_at),
        Duration::from_secs(30),
        "retried 1 ms after the first timeout, timed out again"
    );
    assert!(
        events.all().iter().any(|e| e.starts_with("error:TurnTimeoutException:Rescheduled(")),
        "{:?}",
        events.all()
    );
}
