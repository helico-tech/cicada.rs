//! Example 17 — a purchase that waits for a person's approval, for at most a week; a sign-off by several teams; and
//! the first of several quotes.
//!
//! A flow asks an approvals actor for a decision and waits for it, durably: no handler stays open while a person takes
//! minutes or days to decide. [`Step::request`] sends `Ask` to `approvals/<team>`, created first if it does not exist,
//! with a [`ReplyTo<Decision>`](ReplyTo) route; the request, its entry and the next `Continue` commit together. The
//! [`Step::wait_timeout`] that follows finds no reply yet and saves an open wait with its deadline, a week after the
//! claim of its turn, together with a `Continue` due then; nothing polls meanwhile. The approvals actor keeps the route
//! with the pending purchase. When a person decides, its `Decide` turn replies through the route with
//! [`ReplyTo::send`], in the same commit that removes the pending entry. The run's `Reply` turn keeps the decision,
//! closes the wait as received, cancels the deadline's `Continue` and arms one due now, without running flow code; that
//! `Continue` replays the wait, which returns [`Waited::Received`], and the flow places the order or tells the requester.
//! When nobody decides within the week, the deadline's `Continue` closes the wait as timed out in a turn of its own,
//! and the next one returns [`Waited::TimedOut`]: the flow tells the requester that the purchase expired. The
//! purchase stays pending at the approvals actor, as a timeout cleans nothing up; a decision made after it is ignored
//! by the run.
//!
//! The run accepts only the first reply from the actor its request went to: a reply sent by anyone else, such as a
//! delegate or a direct HTTP send, is ignored. A decision for a run that was deleted rejects the whole `Decide` commit,
//! so the purchase stays pending (see `tests/flow_approval.rs`).
//!
//! Two joins wait for several replies with one wait entry. [`SignOffFlow`] asks the approvals of every team its input
//! lists, in that order, and waits for all decisions for at most a week with [`Step::wait_all_timeout`]: one slot per
//! team, in the order of the input, whichever team decided first, `Received` with the decision or `TimedOut` for a
//! team that had not decided at the deadline. Every slot approved places the order; a rejection or a silent team tells
//! the requester. [`QuoteFlow`] asks every supplier of its input for a quote and takes the first offer with
//! [`Step::wait_first`]: the position of the winning supplier in the input, which stays the same on every replay, and
//! its offer. A losing supplier is not stopped, and its later offer is ignored. Each effect is a step: the code
//! between steps only reads committed data.
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
use cicada_actors::{Flow, FlowRun, ReplyTo, Step, Target, Waited};
use serde::{Deserialize, Serialize};

use crate::steps::ServiceError;

/// The most purchases an approvals actor keeps pending.
pub const MAX_PENDING: usize = 100;

/// How long a purchase waits for a decision before it expires: a week.
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(7 * 24 * 3600);

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

    /// Tells the requester that nobody decided the purchase in time.
    fn expire(&self, item: &str, idempotency_key: &str) -> impl Future<Output = Result<(), ServiceError>> + Send;

    /// Buys the item from `supplier` at the offered price and returns the order's id.
    fn buy(
        &self,
        item: &str,
        supplier: &str,
        cents: u64,
        idempotency_key: &str,
    ) -> impl Future<Output = Result<String, ServiceError>> + Send;
}

/// The purchase flow, `flows.purchase`, calling `orders`.
pub struct PurchaseFlow<O> {
    /// The ordering service.
    pub orders: O,
}

/// The input of a sign-off run: a purchase that every listed team must approve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SignOffInput {
    /// The teams whose approvals decide, at most 100: one request each, in this order.
    pub teams: Vec<String>,
    /// What to buy.
    pub item: String,
    /// Its price in cents.
    pub cents: u64,
}

/// The sign-off flow, `flows.sign-off`, calling `orders`: every team of the input approves, within a week, or the
/// purchase does not go ahead.
pub struct SignOffFlow<O> {
    /// The ordering service.
    pub orders: O,
}

/// The sign-off run `key`, `flows.sign-off/<key>`.
pub fn sign_off<O: Orders>(key: &str) -> FlowRun<SignOffFlow<O>> {
    FlowRun::named(key)
}

impl<O: Orders> Flow for SignOffFlow<O> {
    const TYPE: &'static str = "flows.sign-off";
    type Input = SignOffInput;

    async fn run(&self, step: &mut Step, input: SignOffInput) -> HandlerResult {
        let (orders, item) = (&self.orders, input.item.as_str());
        let empty = ApprovalsState::default();
        let mut requests = Vec::new();
        for team in &input.teams {
            let approvals = Approvals { team: team.clone() };
            let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
            requests.push(step.request(&format!("ask-{team}"), Target::create(&approvals, &empty), ask).await?);
        }
        // One slot per team, in the order of `input.teams`.
        let slots = step.wait_all_timeout("sign-offs", &requests, DECISION_TIMEOUT).await?;
        let rejected = slots.iter().find_map(|slot| match slot {
            Waited::Received(Decision { approved: false, by }) => Some(by.as_str()),
            _ => None,
        });
        if let Some(by) = rejected {
            step.run("notify-rejected", |key| async move { Ok(orders.reject(item, by, &key).await?) }).await?;
        } else if slots.contains(&Waited::TimedOut) {
            step.run("notify-expired", |key| async move { Ok(orders.expire(item, &key).await?) }).await?;
        } else {
            let _order: String =
                step.run("place-order", |key| async move { Ok(orders.place(item, &key).await?) }).await?;
        }
        Ok(())
    }
}

/// A supplier, `suppliers/<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Supplier {
    /// The supplier's name.
    pub name: String,
}

impl ActorRef for Supplier {
    const TYPE: &'static str = "suppliers";
    type Message = SupplierMsg;
}

/// What a supplier offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Offer {
    /// Its price in cents.
    pub cents: u64,
}

/// The messages of a supplier.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum SupplierMsg {
    /// A flow asks for a quote; the offer goes to `reply` once the supplier makes one.
    Quote {
        /// What would be bought.
        item: String,
        /// Where the offer goes.
        reply: ReplyTo<Offer>,
    },
    /// The supplier offers a price for the quote asked by the request `request`.
    Offer {
        /// The id of the request's message.
        request: MessageId,
        /// The price in cents.
        cents: u64,
    },
}

/// The quotes a supplier has not answered yet, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SupplierState {
    /// The routes of the open quotes; at most [`MAX_PENDING`].
    #[serde(default)]
    pub quotes: Vec<ReplyTo<Offer>>,
}

/// The supplier actor: it keeps each quote's route until it offers, then replies through it.
pub struct SupplierActor;

impl Actor for SupplierActor {
    type Ref = Supplier;
    type State = SupplierState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SupplierMsg) -> HandlerResult {
        match message {
            SupplierMsg::Quote { reply, .. } => {
                if turn.state().quotes.contains(&reply) {
                    return Ok(()); // a redelivered request
                }
                if turn.state().quotes.len() >= MAX_PENDING {
                    return Err(turn.fail(format!("at most {MAX_PENDING} quotes wait for an offer")));
                }
                turn.state_mut().quotes.push(reply);
            }
            SupplierMsg::Offer { request, cents } => {
                let Some(at) = turn.state().quotes.iter().position(|q| q.request == request) else {
                    return Err(turn.fail(format!("no quote waits for the request {request}")));
                };
                let quote = turn.state_mut().quotes.remove(at);
                quote.send(turn, &Offer { cents }).await?;
            }
        }
        Ok(())
    }
}

/// The input of a quote run: an item and the suppliers to ask, at most 100.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QuoteInput {
    /// What to buy.
    pub item: String,
    /// The suppliers to ask: one request each, in this order.
    pub suppliers: Vec<String>,
}

/// The quote flow, `flows.quote`, calling `orders`: it buys from the supplier that offers first.
pub struct QuoteFlow<O> {
    /// The ordering service.
    pub orders: O,
}

/// The quote run `key`, `flows.quote/<key>`.
pub fn quote<O: Orders>(key: &str) -> FlowRun<QuoteFlow<O>> {
    FlowRun::named(key)
}

impl<O: Orders> Flow for QuoteFlow<O> {
    const TYPE: &'static str = "flows.quote";
    type Input = QuoteInput;

    async fn run(&self, step: &mut Step, input: QuoteInput) -> HandlerResult {
        let (orders, item) = (&self.orders, input.item.as_str());
        let (empty, mut quotes) = (SupplierState::default(), Vec::new());
        for name in &input.suppliers {
            let supplier = Supplier { name: name.clone() };
            let ask = |reply| SupplierMsg::Quote { item: input.item.clone(), reply };
            quotes.push(step.request(&format!("quote-{name}"), Target::create(&supplier, &empty), ask).await?);
        }
        // The position in `input.suppliers` of the first offer accepted, the same on every replay.
        let (winner, offer) = step.wait_first("first-quote", &quotes).await?;
        let supplier = input.suppliers[winner].as_str();
        let _order: String =
            step.run("order", |key| async move { Ok(orders.buy(item, supplier, offer.cents, &key).await?) }).await?;
        Ok(())
    }
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
        match step.wait_timeout("decision", &request, DECISION_TIMEOUT).await? {
            Waited::Received(Decision { approved: true, .. }) => {
                let _order: String =
                    step.run("place-order", |key| async move { Ok(orders.place(item, &key).await?) }).await?;
            }
            Waited::Received(Decision { approved: false, by }) => {
                let by = by.as_str();
                step.run("notify-rejected", |key| async move { Ok(orders.reject(item, by, &key).await?) }).await?;
            }
            Waited::TimedOut => {
                step.run("notify-expired", |key| async move { Ok(orders.expire(item, &key).await?) }).await?;
            }
        }
        Ok(())
    }
}
