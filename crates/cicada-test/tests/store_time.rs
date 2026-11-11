//! A host reads the store's time and the next due time through the system: `ActorSystem::now` and
//! `ActorSystem::next_due_at`.

use std::time::Duration;

use cicada::SendOptions;
use cicada::prelude::*;
use cicada::time;
use cicada_test::TestActors;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Bell {
    id: String,
}

impl ActorRef for Bell {
    const TYPE: &'static str = "bell";
    type Message = BellMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum BellMsg {
    Ring,
}

struct BellActor;

impl Actor for BellActor {
    type Ref = Bell;
    type State = NoState;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: BellMsg) -> HandlerResult {
        Ok(())
    }
}

fn bell() -> Bell {
    Bell { id: "b".into() }
}

async fn harness() -> TestActors {
    let test = TestActors::builder().with(|node| node.host(BellActor)).build().await.unwrap();
    test.create(&bell(), &NoState {}).await;
    test
}

#[tokio::test(start_paused = true)]
async fn now_is_the_stores_time_and_moves_with_virtual_time() {
    let test = harness().await;
    let before = test.system().now().await.unwrap();
    assert_eq!(before, test.now());

    test.advance_time_by(Duration::from_secs(3600)).await;

    assert_eq!(test.system().now().await.unwrap(), time::add(before, Duration::from_secs(3600)).unwrap());
}

#[tokio::test(start_paused = true)]
async fn next_due_at_is_the_delivery_time_of_a_delayed_message() {
    let test = harness().await;
    let delay = Duration::from_secs(10 * 60);

    test.send_with(&bell(), BellMsg::Ring, SendOptions::new().delay(delay)).await;

    assert_eq!(test.system().next_due_at().await.unwrap(), Some(time::add(test.now(), delay).unwrap()));
}

#[tokio::test(start_paused = true)]
async fn next_due_at_is_none_when_nothing_is_pending() {
    let test = harness().await;
    assert_eq!(test.system().next_due_at().await.unwrap(), None);

    test.send(&bell(), BellMsg::Ring).await;
    test.run_until_idle().await;

    assert_eq!(test.system().next_due_at().await.unwrap(), None);
}

#[tokio::test(start_paused = true)]
async fn next_due_at_leaves_out_a_paused_actor() {
    let test = harness().await;
    let address = bell().address().unwrap();
    assert!(test.system().pause_mailbox(&address).await.unwrap());

    test.send(&bell(), BellMsg::Ring).await;

    assert_eq!(test.system().next_due_at().await.unwrap(), None);
    assert!(test.system().resume_mailbox(&address).await.unwrap());
    assert_eq!(test.system().next_due_at().await.unwrap(), Some(test.now()));
}
