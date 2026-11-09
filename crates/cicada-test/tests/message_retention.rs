//! The sender's retention policy, from outside and from a handler, in virtual time.

use std::time::Duration;

use cicada::prelude::*;
use cicada::store::MessageStatus;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Keeper {
    id: String,
}

impl ActorRef for Keeper {
    const TYPE: &'static str = "keeper";
    type Message = KeeperMsg;
}

fn keeper() -> Keeper {
    Keeper { id: "k".into() }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum KeeperMsg {
    /// Sends a `Note` to itself with the policy given, so the turn's own send carries it.
    Pass { retention: Option<Retention> },
    Note {
        #[serde(default)]
        fail: bool,
    },
}

struct KeeperActor;

impl Actor for KeeperActor {
    type Ref = Keeper;
    type State = Stateless;
    type Message = KeeperMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: KeeperMsg) -> HandlerResult {
        match message {
            KeeperMsg::Pass { retention } => {
                let mut options = SendOptions::new();
                options.retention = retention;
                turn.send_self_with(KeeperMsg::Note { fail: false }, options)?;
            }
            KeeperMsg::Note { fail } => {
                if fail {
                    return Err(HandlerError::msg("note failed"));
                }
            }
        }
        Ok(())
    }
}

async fn harness() -> TestActors {
    let test = TestActors::builder().with(|n| n.host(KeeperActor)).build().await.unwrap();
    test.create_stateless(&keeper()).await;
    test
}

#[tokio::test(start_paused = true)]
async fn a_handlers_send_carries_its_retention_policy_and_a_removed_record_has_no_status() {
    let test = harness().await;
    let pass = test.send(&keeper(), KeeperMsg::Pass { retention: Some(Retention::REMOVE_ON_DONE) }).await;
    assert_eq!(test.system().run_due_turns(1).await.unwrap(), 1);
    let notes = test.pending(&keeper()).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].retention, Some(Retention::REMOVE_ON_DONE));
    assert_eq!(notes[0].caused_by.as_ref(), Some(&pass.id));
    assert_eq!(test.run_until_idle().await, 1);
    assert!(test.system().status(&notes[0].id).await.unwrap().is_none());
    assert!(
        matches!(test.system().status(&pass.id).await.unwrap(), Some(MessageStatus::Finished(_))),
        "the sender's own message keeps the default"
    );
}

#[tokio::test(start_paused = true)]
async fn a_record_kept_for_a_time_disappears_after_it_and_a_failed_one_follows_the_failed_rule() {
    let test = harness().await;
    let hour = Duration::from_secs(3600);
    let day = 24 * hour;
    let policy =
        Retention { done: Some(Keep::keep_for(hour).unwrap()), failed: Some(Keep::keep_for(30 * day).unwrap()) };
    let done =
        test.send_with(&keeper(), KeeperMsg::Note { fail: false }, SendOptions::new().retention(policy.clone())).await;
    let failed = test.send_with(&keeper(), KeeperMsg::Note { fail: true }, SendOptions::new().retention(policy)).await;
    test.run_until_idle().await;
    assert!(test.outcome(&done).await.is_some());
    test.advance_time_by(2 * hour).await;
    assert!(test.outcome(&done).await.is_none());
    assert_eq!(test.outcome(&failed).await.unwrap().error.as_deref(), Some("note failed"));
    test.advance_time_by(10 * day).await; // past the default retention of 7 days
    assert_eq!(test.failures().await.into_iter().map(|f| f.message_id).collect::<Vec<_>>(), vec![failed.id]);
}
