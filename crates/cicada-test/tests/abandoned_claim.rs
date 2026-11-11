//! `claim_and_abandon` plays a node that claims a turn and dies: the lease runs out and a running node handles the
//! message as its second attempt.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::prelude::*;
use cicada::store::OutcomeStatus;
use cicada_test::{TestActors, claim_and_abandon};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Job {
    id: String,
}

impl ActorRef for Job {
    const TYPE: &'static str = "job";
    type Message = JobMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum JobMsg {
    Run,
}

/// Records the attempt of every handler run.
struct JobActor(Arc<Mutex<Vec<u32>>>);

impl Actor for JobActor {
    type Ref = Job;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, _message: JobMsg) -> HandlerResult {
        self.0.lock().unwrap().push(turn.message().attempt);
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_node_that_claims_a_turn_and_dies_leaves_it_to_a_running_node_as_its_second_attempt() {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let actor = JobActor(attempts.clone());
    let test = TestActors::builder().with(|node| node.host(actor)).build().await.unwrap();
    let job = Job { id: "j1".into() };
    test.create(&job, &NoState {}).await;
    let message = test.send(&job, JobMsg::Run).await;
    let lease = Duration::from_secs(30);

    let abandoned = claim_and_abandon(test.store(), &test.system().hosted_types(), lease, 10).await.unwrap();

    assert_eq!(abandoned.len(), 1);
    assert_eq!(abandoned[0].address, job.address().unwrap());
    assert_eq!(abandoned[0].message_id, message.id);
    assert_eq!(test.run_until_idle().await, 0, "the dead node's lease is still valid");
    assert_eq!(test.advance_time_by(lease + Duration::from_secs(1)).await, 1);
    assert_eq!(*attempts.lock().unwrap(), vec![2]);
    let outcome = test.outcome(&message).await.unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Done);
    assert_eq!(outcome.envelope.attempt, 2);
}
