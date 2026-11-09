//! A turn cancels pending messages of its own instance, atomically with its commit.

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::{MessageId, OutcomeStatus};
use cicada::time;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Alarm {
    id: String,
}

impl ActorRef for Alarm {
    const TEMPLATE: &'static str = "actor://alarm/{id}";
    type Message = AlarmMsg;
}

fn alarm() -> Alarm {
    Alarm { id: "a".into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum AlarmMsg {
    /// Replaces the armed `Ring` with one due in `minutes`. With `fail`, the turn fails after cancelling.
    Arm {
        minutes: u64,
        #[serde(default)]
        fail: bool,
    },
    Ring,
    /// Tries to cancel its own message.
    CancelSelf,
    /// Sends a `Ring` and cancels it in the same turn.
    SendThenCancel,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
struct AlarmState {
    armed: Option<String>,
    #[serde(default)]
    rings: u32,
    refused: Option<String>,
}

struct AlarmActor;

impl Actor for AlarmActor {
    type Ref = Alarm;
    type State = AlarmState;
    type Message = AlarmMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: AlarmMsg) -> HandlerResult {
        match message {
            AlarmMsg::Arm { minutes, fail } => {
                if let Some(armed) = turn.state().armed.clone() {
                    turn.cancel(&MessageId::new(armed).map_err(HandlerError::msg)?)?;
                }
                let id =
                    turn.send_self_with(AlarmMsg::Ring, SendOptions::new().delay(Duration::from_secs(minutes * 60)))?;
                turn.state_mut().armed = Some(id.to_string());
                if fail {
                    return Err(HandlerError::msg("arming failed"));
                }
            }
            AlarmMsg::Ring => {
                let state = turn.state_mut();
                state.rings += 1;
                state.armed = None;
            }
            AlarmMsg::CancelSelf => {
                let own = turn.message().id.clone();
                if let Err(e) = turn.cancel(&own) {
                    turn.state_mut().refused = Some(e.to_string());
                }
            }
            AlarmMsg::SendThenCancel => {
                let id = turn.send_self(AlarmMsg::Ring)?;
                turn.cancel(&id)?;
            }
        }
        Ok(())
    }
}

async fn harness() -> TestActors {
    let test = TestActors::builder().with(|n| n.host(AlarmActor)).build().await.unwrap();
    test.create(&alarm(), &AlarmState::default()).await;
    test
}

#[tokio::test(start_paused = true)]
async fn re_arming_replaces_the_pending_message_so_exactly_one_is_armed() {
    let test = harness().await;
    test.send(&alarm(), AlarmMsg::Arm { minutes: 60, fail: false }).await;
    test.run_until_idle().await;
    let first = test.pending(&alarm()).await;
    assert_eq!(first.len(), 1);
    test.send(&alarm(), AlarmMsg::Arm { minutes: 10, fail: false }).await;
    test.run_until_idle().await;
    let second = test.pending(&alarm()).await;
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].deliver_at, time::add(test.now(), Duration::from_secs(600)).unwrap());
    assert_eq!(test.outcome(&first[0]).await.map(|o| o.status), Some(OutcomeStatus::Cancelled));
    test.advance_time_by(Duration::from_secs(2 * 3600)).await;
    assert_eq!(test.state::<_, AlarmState>(&alarm()).await.rings, 1, "only the replacement rang");
}

#[tokio::test(start_paused = true)]
async fn a_failed_turn_cancels_nothing() {
    let test = harness().await;
    test.send(&alarm(), AlarmMsg::Arm { minutes: 60, fail: false }).await;
    test.run_until_idle().await;
    let armed = test.pending(&alarm()).await.remove(0);
    test.send(&alarm(), AlarmMsg::Arm { minutes: 10, fail: true }).await;
    test.run_until_idle().await;
    assert_eq!(test.pending(&alarm()).await.into_iter().map(|m| m.id).collect::<Vec<_>>(), vec![armed.id.clone()]);
    assert!(test.outcome(&armed).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_turn_cannot_cancel_its_own_message() {
    let test = harness().await;
    let m = test.send(&alarm(), AlarmMsg::CancelSelf).await;
    test.run_until_idle().await;
    assert_eq!(test.outcome(&m).await.map(|o| o.status), Some(OutcomeStatus::Done));
    let refused = test.state::<_, AlarmState>(&alarm()).await.refused.unwrap();
    assert!(refused.starts_with("A turn cannot cancel its own message"), "{refused}");
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_message_sent_in_the_same_turn_does_not_send_it() {
    let test = harness().await;
    test.send(&alarm(), AlarmMsg::SendThenCancel).await;
    test.run_until_idle().await;
    assert!(test.pending(&alarm()).await.is_empty());
    assert_eq!(test.state::<_, AlarmState>(&alarm()).await.rings, 0);
}
