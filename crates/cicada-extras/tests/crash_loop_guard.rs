//! The crash-loop guard.

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada_extras::CrashLoopGuard;
use cicada_test::{TestActors, claim_and_abandon};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Guarded {
    id: String,
}

impl ActorRef for Guarded {
    const TYPE: &'static str = "guarded";
    type Message = GuardedMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum GuardedMsg {
    Work,
    Boom,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct GuardedState {
    runs: u32,
}

struct GuardedActor;

impl Actor for GuardedActor {
    type Ref = Guarded;
    type State = GuardedState;

    async fn handle(&self, turn: &mut Turn<Self>, message: GuardedMsg) -> HandlerResult {
        match message {
            GuardedMsg::Work => turn.state_mut().runs += 1,
            GuardedMsg::Boom => return Err(HandlerError::msg("boom")),
        }
        Ok(())
    }
}

fn guarded() -> Guarded {
    Guarded { id: "a".into() }
}

async fn harness(limit: u32) -> TestActors {
    TestActors::builder()
        .with(move |node| node.host(GuardedActor).install(CrashLoopGuard::new(limit).unwrap()))
        .build()
        .await
        .unwrap()
}

/// A node that claims the head message and dies before committing: its lease simply expires.
async fn crash(test: &TestActors) {
    let claims = claim_and_abandon(test.store(), &test.system().hosted_types(), Duration::from_secs(30), 1).await;
    assert_eq!(claims.unwrap().len(), 1);
    tokio::time::sleep(Duration::from_secs(31)).await; // only time passes: no turn runs meanwhile
}

#[tokio::test(start_paused = true)]
async fn a_message_claimed_too_often_without_a_commit_ends_failed_without_running_the_handler() {
    let test = harness(3).await;
    test.create(&guarded(), &GuardedState::default()).await;
    let msg = test.send(&guarded(), GuardedMsg::Work).await;
    for _ in 0..3 {
        crash(&test).await;
    }
    test.run_until_idle().await;
    let outcome = test.outcome(&msg).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert_eq!(
        outcome.error.as_deref(),
        Some("on_claim of plugin CrashLoopGuard failed: crash loop: 3 claims without a commit")
    );
    assert_eq!(test.state::<_, GuardedState>(&guarded()).await.runs, 0);
}

#[tokio::test(start_paused = true)]
async fn below_the_limit_the_message_is_handled() {
    let test = harness(3).await;
    test.create(&guarded(), &GuardedState::default()).await;
    let msg = test.send(&guarded(), GuardedMsg::Work).await;
    for _ in 0..2 {
        crash(&test).await;
    }
    test.run_until_idle().await;
    assert_eq!(test.outcome(&msg).await.unwrap().status, OutcomeStatus::Done);
    assert_eq!(test.state::<_, GuardedState>(&guarded()).await.runs, 1);
}

#[tokio::test(start_paused = true)]
async fn committed_retries_do_not_count() {
    let test = harness(1).await;
    test.create(&guarded(), &GuardedState::default()).await;
    let retry = RetryPolicy::new(4, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    let msg = test.send_with(&guarded(), GuardedMsg::Boom, SendOptions::new().retry(retry)).await;
    test.advance_time_by(Duration::from_secs(10)).await;
    let outcome = test.outcome(&msg).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert_eq!(outcome.envelope.attempt, 4);
    assert!(outcome.error.as_deref().unwrap().contains("boom"), "{:?}", outcome.error);
}

#[test]
fn the_limit_must_be_at_least_1() {
    assert_eq!(CrashLoopGuard::new(0).unwrap_err(), "max_uncommitted_claims must be at least 1, was 0");
    assert_eq!(CrashLoopGuard::default().max_uncommitted_claims(), 5);
}
