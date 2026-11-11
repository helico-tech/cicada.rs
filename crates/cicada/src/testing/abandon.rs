//! A node that claims a turn and dies.

use std::collections::BTreeSet;
use std::time::Duration;

use crate::address::{ActorAddress, TypeName};
use crate::store::{Backend, MessageId, NodeId, Store, StoreResult};

/// A turn claimed by [`claim_and_abandon`] and never committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbandonedClaim {
    /// The actor whose lease the dead node holds until it runs out.
    pub address: ActorAddress,
    /// The claimed message; it stays pending.
    pub message_id: MessageId,
}

/// Plays a node that claims a turn and dies: claims up to `limit` due messages of `types` on `store` as a node of its
/// own, with a lease of `lease`, and commits nothing. The messages stay pending; once the lease runs out a running node
/// claims each again, as its next attempt, as after a crash. Works on every backend.
///
/// ```
/// # use std::time::Duration;
/// # use cicada::prelude::*;
/// # use serde::{Deserialize, Serialize};
/// # #[derive(Debug, Clone, Serialize, Deserialize)]
/// # struct Job { id: String }
/// # impl ActorRef for Job { const TYPE: &'static str = "job"; type Message = JobMsg; }
/// # #[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// # #[serde(tag = "type")]
/// # enum JobMsg { Run }
/// # struct JobActor;
/// # impl Actor for JobActor {
/// #     type Ref = Job; type State = NoState; type Message = JobMsg;
/// #     const UNHANDLED: Unhandled = Unhandled::Fail;
/// #     async fn handle(&self, _: &mut Turn<Self>, _: JobMsg) -> HandlerResult { Ok(()) }
/// # }
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() {
/// use cicada::testing::{TestActors, claim_and_abandon};
///
/// let test = TestActors::builder().with(|node| node.host(JobActor)).build().await.unwrap();
/// let job = Job { id: "j1".into() };
/// test.create(&job, &NoState {}).await;
/// let message = test.send(&job, JobMsg::Run).await;
/// let lease = Duration::from_secs(30);
/// let abandoned = claim_and_abandon(test.store(), &test.system().hosted_types(), lease, 1).await.unwrap();
/// assert_eq!(abandoned[0].message_id, message.id);
///
/// test.advance_time_by(lease + Duration::from_secs(1)).await; // the lease runs out; the node runs the turn
/// assert_eq!(test.outcome(&message).await.unwrap().envelope.attempt, 2);
/// # }
/// ```
pub async fn claim_and_abandon(
    store: &Store,
    types: &BTreeSet<TypeName>,
    lease: Duration,
    limit: usize,
) -> StoreResult<Vec<AbandonedClaim>> {
    let claims = store.claim_due(types, &NodeId::random(), lease, limit).await?;
    Ok(claims.into_iter().map(|c| AbandonedClaim { address: c.actor, message_id: c.message.id }).collect())
}
