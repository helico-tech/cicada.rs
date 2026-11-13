//! Example 14 — flows started by an event.
//!
//! A placed order starts two independent jobs: a receipt mail and a fulfilment. Each is a [`Flow`] with the typed input
//! [`PlacedOrder`], and the event [`order_placed`], an [`EventActor`](cicada_actors::EventActor), starts one run of
//! each for every publication, in one commit: `orders.receipt/<publication id>` and `orders.fulfil/<publication id>`.
//! A publication whose runs cannot all start (a type that is not registered, input that a hosting node cannot decode)
//! starts none of them. The orders also go, one by one, to the batcher [`orders_batcher`], whose message type is
//! `PublishBatch`: each batch of up to 20 orders, or what came within an hour, starts one run of the digest flow with
//! the input [`Digest`] `{"items": [...]}`, through the event [`orders_digest`].
//!
//! The host creates the events, subscribes the flows and sets the watches at its start ([`set_up`]); nothing subscribes
//! itself. Dead letters keep a failed publication and a run that ran out of attempts; a run is recovered with a
//! public `Continue`, a publication by a redrive, which starts its runs under the redrive's new id (see
//! `tests/flow_events.rs`).
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::{BatcherActor, DeadLettersActor, EventActor, EventMsg, FlowActor};
//! # use cicada_examples::flow_events::{
//! #     DigestFlow, FulfilFlow, PlacedOrder, ReceiptFlow, ShopServices, order_placed, set_up,
//! # };
//! # async fn run<S: ShopServices + Clone>(store: cicada::Store, shop: S) -> Result<(), Box<dyn std::error::Error>> {
//! let system = ActorSystem::builder(store)
//!     .host(EventActor)
//!     .host(BatcherActor)
//!     .host(DeadLettersActor::default())
//!     .host(FlowActor::new(ReceiptFlow { services: shop.clone() }))
//!     .host(FlowActor::new(FulfilFlow { services: shop.clone() }))
//!     .host(FlowActor::new(DigestFlow { services: shop }))
//!     .build();
//! system.start().await?;
//! set_up(&system).await?;
//! let order = PlacedOrder { order: "o-1".into(), email: "ann@example.test".into(), cents: 1250 };
//! system.send(&order_placed(), EventMsg::publish(&order)?).await?;
//! # Ok(()) }
//! ```

use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use cicada::{OpError, WatchTarget};
use cicada_actors::{
    Batcher, BatcherState, DeadLetters, DeadLettersState, Event, EventMsg, EventState, Flow, FlowRun, Step,
};
use serde::{Deserialize, Serialize};

use crate::steps::ServiceError;

/// The event of a placed order, `cicada.event/order-placed`.
pub fn order_placed() -> Event {
    Event::named("order-placed")
}

/// The event of a batch of orders, `cicada.event/orders-digest`.
pub fn orders_digest() -> Event {
    Event::named("orders-digest")
}

/// The batcher of the orders, `cicada.batcher/orders`.
pub fn orders_batcher() -> Batcher {
    Batcher::named("orders")
}

/// A `PublishBatch` to [`orders_digest`] of at most 20 orders, at most an hour from the first.
pub fn orders_batcher_settings() -> BatcherState {
    BatcherState::new(&orders_digest(), "PublishBatch", 20, Duration::from_secs(3600)).expect("valid settings")
}

/// Creates the events, the batcher and the dead letters, subscribes the flows, and has the dead letters watch the
/// events and the flows. Safe to repeat: a create of an actor that exists and a subscription held already change
/// nothing.
pub async fn set_up(system: &ActorSystem) -> Result<(), OpError> {
    system.create(&order_placed(), &EventState::default()).await?;
    system.create(&orders_digest(), &EventState::default()).await?;
    system.create(&orders_batcher(), &orders_batcher_settings()).await?;
    system.create(&DeadLetters::default(), &DeadLettersState::default()).await?;
    for (event, flow) in [(order_placed(), RECEIPT), (order_placed(), FULFIL), (orders_digest(), DIGEST)] {
        system.send(&event, EventMsg::Subscribe { flow: flow.to_owned() }).await?;
    }
    let letters = DeadLetters::default().address().expect("a valid address");
    for target in ["cicada.event", RECEIPT, FULFIL, DIGEST] {
        system.watch(&WatchTarget::parse(target).expect("a valid target"), &letters).await?;
    }
    Ok(())
}

/// The type name of the receipt flow.
pub const RECEIPT: &str = "orders.receipt";
/// The type name of the fulfilment flow.
pub const FULFIL: &str = "orders.fulfil";
/// The type name of the digest flow.
pub const DIGEST: &str = "orders.digest";

/// A placed order: the input of a receipt and of a fulfilment, and an item of a digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlacedOrder {
    /// The order id.
    pub order: String,
    /// The customer's mail address.
    pub email: String,
    /// The total, in cents.
    pub cents: u64,
}

/// The input of a digest: the orders of one batch, in the field `items` that a `PublishBatch` fills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Digest {
    /// The orders of the batch.
    pub items: Vec<PlacedOrder>,
}

/// The services of the shop. Each call is at least once and receives an idempotency key.
pub trait ShopServices: Send + Sync + 'static {
    /// Mails the receipt of `order`.
    fn send_receipt(
        &self,
        order: &PlacedOrder,
        idempotency_key: &str,
    ) -> impl Future<Output = Result<(), ServiceError>> + Send;

    /// Ships `order` and returns its tracking number.
    fn ship(
        &self,
        order: &PlacedOrder,
        idempotency_key: &str,
    ) -> impl Future<Output = Result<String, ServiceError>> + Send;

    /// Writes the digest of `orders`.
    fn write_digest(
        &self,
        orders: &[PlacedOrder],
        idempotency_key: &str,
    ) -> impl Future<Output = Result<(), ServiceError>> + Send;
}

/// The receipt flow, `orders.receipt`: one step.
pub struct ReceiptFlow<S> {
    /// The services.
    pub services: S,
}

impl<S: ShopServices> Flow for ReceiptFlow<S> {
    const TYPE: &'static str = RECEIPT;
    type Input = PlacedOrder;

    async fn run(&self, step: &mut Step, order: PlacedOrder) -> HandlerResult {
        let (services, order) = (&self.services, &order);
        step.run("send-receipt", |key| async move { Ok(services.send_receipt(order, &key).await?) }).await
    }
}

/// The fulfilment flow, `orders.fulfil`: ships the order, three attempts five seconds apart.
pub struct FulfilFlow<S> {
    /// The services.
    pub services: S,
}

impl<S: ShopServices> Flow for FulfilFlow<S> {
    const TYPE: &'static str = FULFIL;
    type Input = PlacedOrder;

    fn retry() -> RetryPolicy {
        RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5)).expect("a valid backoff")).expect("a valid policy")
    }

    async fn run(&self, step: &mut Step, order: PlacedOrder) -> HandlerResult {
        let (services, order) = (&self.services, &order);
        let _tracking: String = step.run("ship", |key| async move { Ok(services.ship(order, &key).await?) }).await?;
        Ok(())
    }
}

/// The digest flow, `orders.digest`: one step for the whole batch.
pub struct DigestFlow<S> {
    /// The services.
    pub services: S,
}

impl<S: ShopServices> Flow for DigestFlow<S> {
    const TYPE: &'static str = DIGEST;
    type Input = Digest;

    async fn run(&self, step: &mut Step, digest: Digest) -> HandlerResult {
        let (services, items) = (&self.services, digest.items.as_slice());
        step.run("write-digest", |key| async move { Ok(services.write_digest(items, &key).await?) }).await
    }
}

/// The receipt run that the publication `publication` started.
pub fn receipt<S: ShopServices>(publication: &str) -> FlowRun<ReceiptFlow<S>> {
    FlowRun::named(publication)
}

/// The fulfilment run that the publication `publication` started.
pub fn fulfilment<S: ShopServices>(publication: &str) -> FlowRun<FulfilFlow<S>> {
    FlowRun::named(publication)
}

/// The digest run that the batch `publication` started.
pub fn digest<S: ShopServices>(publication: &str) -> FlowRun<DigestFlow<S>> {
    FlowRun::named(publication)
}
