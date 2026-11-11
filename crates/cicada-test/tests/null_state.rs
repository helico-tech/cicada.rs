//! A state is never `null`: a create in a turn fails at the call, and a turn that ends with a `null` state fails its
//! message without a retry and keeps the stored state.

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada::turn::TurnError;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

/// type `drafts`, key `{id}`: its state is an optional draft, so `None` encodes to `null`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Drafts {
    id: String,
}

impl ActorRef for Drafts {
    const TYPE: &'static str = "drafts";
    type Message = DraftsMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum DraftsMsg {
    /// Creates `drafts/{id}` with the state `()`, which encodes to `null`, and keeps the error of the call.
    CreateUnit { id: String },
    /// Sets the own state to `None`.
    Discard,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Draft {
    text: String,
}

struct DraftsActor;

impl Actor for DraftsActor {
    type Ref = Drafts;
    type State = Option<Draft>;
    type Message = DraftsMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: DraftsMsg) -> HandlerResult {
        match message {
            DraftsMsg::CreateUnit { id } => match turn.create(&Drafts { id }, &()) {
                Err(TurnError::Invalid(e)) => turn.set_state(Some(Draft { text: e })),
                other => return Err(HandlerError::msg(format!("expected TurnError::Invalid, got {other:?}"))),
            },
            DraftsMsg::Discard => turn.set_state(None),
        }
        Ok(())
    }
}

fn drafts(id: &str) -> Drafts {
    Drafts { id: id.into() }
}

fn draft(text: &str) -> Option<Draft> {
    Some(Draft { text: text.into() })
}

async fn harness() -> TestActors {
    let test = TestActors::builder().with(|n| n.host(DraftsActor)).build().await.unwrap();
    test.create(&drafts("a"), &draft("hello")).await;
    test
}

#[tokio::test(start_paused = true)]
async fn a_turn_that_creates_with_a_null_state_fails_at_the_call_and_creates_nothing() {
    let test = harness().await;
    let m = test.send(&drafts("a"), DraftsMsg::CreateUnit { id: "unit".into() }).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.unwrap().status, OutcomeStatus::Done, "the handler handled the error");
    let kept = test.state::<_, Option<Draft>>(&drafts("a")).await.unwrap();
    assert!(kept.text.contains("never null"), "{kept:?}");
    assert!(!test.exists(&drafts("unit")).await);
}

#[tokio::test(start_paused = true)]
async fn a_turn_that_sets_a_null_state_fails_its_message_without_a_retry_and_keeps_the_state() {
    let test = harness().await;
    let retry = RetryPolicy::new(5, Backoff::fixed(Duration::from_secs(1)).unwrap()).unwrap();
    let m = test.send_with(&drafts("a"), DraftsMsg::Discard, SendOptions::new().retry(retry)).await;
    test.run_until_idle().await;
    test.advance_time_by(Duration::from_secs(60)).await;
    let outcome = test.outcome(&m).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Failed);
    assert_eq!(outcome.envelope.attempt, 1, "not retried");
    assert!(outcome.error.as_deref().unwrap().contains("never null"), "{:?}", outcome.error);
    assert_eq!(test.state::<_, Option<Draft>>(&drafts("a")).await, draft("hello"));
    assert_eq!(test.version(&drafts("a")).await, 0);
}
