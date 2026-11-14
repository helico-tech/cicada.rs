//! Example 17 — a purchase that waits for a person's approval.
//!
//! A flow asks an approvals actor for a decision and waits for it, durably: no handler stays open while a person takes
//! minutes or days to decide. [`Step::request`] sends `Ask` to `approvals/<team>`, created first if it does not exist,
//! with a [`ReplyTo<Decision>`](ReplyTo) route; the request, its entry and the next `Continue` commit together. The
//! [`Step::wait`] that follows finds no reply yet, saves an open wait and parks the run: no `Continue` is armed and
//! nothing polls. The approvals actor keeps the route with the pending purchase. When a person decides, its `Decide`
//! turn replies through the route with [`ReplyTo::send`], in the same commit that removes the pending entry. The run's
//! `Reply` turn keeps the decision, closes the wait and arms a `Continue`, without running flow code; that `Continue`
//! replays the wait, which returns the decision, and the flow places the order or tells the requester.
//!
//! The run accepts only the first reply from the actor its request went to: a reply sent by anyone else, such as a
//! delegate or a direct HTTP send, is ignored. A decision for a run that was deleted rejects the whole `Decide` commit,
//! so the purchase stays pending (see `tests/flow_approval.rs`). The wait has no timeout: a purchase nobody decides
//! leaves its run parked.
//!
//! A host registers both types, creates a run and sends it `Continue`:
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::{FlowActor, FlowMsg, FlowState};
//! # use cicada_examples::flow_approval::{ApprovalsActor, Orders, PurchaseFlow, PurchaseInput, purchase};
//! # async fn run<O: Orders>(store: cicada::Store, orders: O) -> Result<(), Box<dyn std::error::Error>> {
//! let system = ActorSystem::builder(store).host(ApprovalsActor).host(FlowActor::new(PurchaseFlow { orders })).build();
//! system.start().await?;
//! let run = purchase::<O>("p-17");
//! let input = PurchaseInput { team: "design".into(), item: "monitor".into(), cents: 39_900 };
//! system.create(&run, &FlowState::new(input)).await?;
//! system.send(&run, FlowMsg::Continue).await?;
//! # Ok(()) }
//! ```

use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use cicada_actors::{Flow, FlowRun, ReplyTo, Step, Target};
use serde::{Deserialize, Serialize};

use crate::steps::ServiceError;

/// The most purchases an approvals actor keeps pending.
pub const MAX_PENDING: usize = 100;

/// The approvals of a team, `approvals/<team>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approvals {
    /// The team.
    pub team: String,
}

impl ActorRef for Approvals {
    const TYPE: &'static str = "approvals";
    type Message = ApprovalsMsg;
}

/// What a person decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Decision {
    /// Whether the purchase may go ahead.
    pub approved: bool,
    /// Who decided.
    pub by: String,
}

/// The messages of the approvals of a team.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ApprovalsMsg {
    /// A flow asks for a decision on a purchase; the reply goes to `reply` once a person decides.
    Ask {
        /// What would be bought.
        item: String,
        /// Its price in cents.
        cents: u64,
        /// Where the decision goes.
        reply: ReplyTo<Decision>,
    },
    /// A person decides the purchase asked by the request `request`.
    Decide {
        /// The id of the request's message, as the pending purchase shows it.
        request: MessageId,
        /// Whether the purchase may go ahead.
        approved: bool,
        /// Who decided.
        by: String,
    },
}

/// A purchase that waits for a decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pending {
    /// What would be bought.
    pub item: String,
    /// Its price in cents.
    pub cents: u64,
    /// Where the decision goes.
    pub reply: ReplyTo<Decision>,
}

/// The purchases a team has not decided yet, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalsState {
    /// The pending purchases; at most [`MAX_PENDING`].
    #[serde(default)]
    pub pending: Vec<Pending>,
}

/// The approvals actor: it keeps each request's route until a person decides, then replies through it.
pub struct ApprovalsActor;

impl Actor for ApprovalsActor {
    type Ref = Approvals;
    type State = ApprovalsState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ApprovalsMsg) -> HandlerResult {
        match message {
            ApprovalsMsg::Ask { item, cents, reply } => {
                if turn.state().pending.iter().any(|p| p.reply == reply) {
                    return Ok(()); // a redelivered request
                }
                if turn.state().pending.len() >= MAX_PENDING {
                    return Err(turn.fail(format!("at most {MAX_PENDING} purchases wait for a decision")));
                }
                turn.state_mut().pending.push(Pending { item, cents, reply });
            }
            ApprovalsMsg::Decide { request, approved, by } => {
                let Some(at) = turn.state().pending.iter().position(|p| p.reply.request == request) else {
                    return Err(turn.fail(format!("no purchase waits for the request {request}")));
                };
                let decided = turn.state_mut().pending.remove(at);
                decided.reply.send(turn, &Decision { approved, by }).await?;
            }
        }
        Ok(())
    }
}

/// The input of a purchase run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PurchaseInput {
    /// The team whose approvals decide.
    pub team: String,
    /// What to buy.
    pub item: String,
    /// Its price in cents.
    pub cents: u64,
}

/// The ordering service the flow calls. Each call is at least once and receives an idempotency key.
pub trait Orders: Send + Sync + 'static {
    /// Places the order and returns its id.
    fn place(&self, item: &str, idempotency_key: &str) -> impl Future<Output = Result<String, ServiceError>> + Send;

    /// Tells the requester that `by` rejected the purchase.
    fn reject(
        &self,
        item: &str,
        by: &str,
        idempotency_key: &str,
    ) -> impl Future<Output = Result<(), ServiceError>> + Send;
}

/// The purchase flow, `flows.purchase`, calling `orders`.
pub struct PurchaseFlow<O> {
    /// The ordering service.
    pub orders: O,
}

/// The purchase run `key`, `flows.purchase/<key>`.
pub fn purchase<O: Orders>(key: &str) -> FlowRun<PurchaseFlow<O>> {
    FlowRun::named(key)
}

impl<O: Orders> Flow for PurchaseFlow<O> {
    const TYPE: &'static str = "flows.purchase";
    type Input = PurchaseInput;

    fn retry() -> RetryPolicy {
        RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5)).expect("a valid backoff")).expect("a valid policy")
    }

    async fn run(&self, step: &mut Step, input: PurchaseInput) -> HandlerResult {
        let (orders, item) = (&self.orders, input.item.as_str());
        let (approvals, empty) = (Approvals { team: input.team.clone() }, ApprovalsState::default());
        let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
        let request = step.request("ask-approval", Target::create(&approvals, &empty), ask).await?;
        let decision = step.wait("decision", &request).await?;
        if decision.approved {
            let _order: String =
                step.run("place-order", |key| async move { Ok(orders.place(item, &key).await?) }).await?;
        } else {
            let by = decision.by.as_str();
            step.run("notify-rejected", |key| async move { Ok(orders.reject(item, by, &key).await?) }).await?;
        }
        Ok(())
    }
}
