//! Actor change hints (ADR 0034): a best-effort feed that tells a host which actors may have changed, so that it can
//! reread them instead of polling every row.
//!
//! [`ActorSystem::changes`](crate::ActorSystem::changes) returns an [`ActorChanges`] stream of [`ActorChange`] items:
//! [`ActorChange::Changed`] names an address whose snapshot may have changed, and [`ActorChange::Resync`] asks for a
//! full reread of the prefix. The first item is always `Resync`, once the store's source is live. A hint carries no
//! state, version or kind of change: the host reads the actor after it, and the read is the truth.
//!
//! ```no_run
//! # async fn example(system: cicada::ActorSystem) -> Result<(), cicada::OpError> {
//! use std::collections::BTreeMap;
//!
//! use cicada::ActorChange;
//! use futures::StreamExt;
//!
//! let mut view = BTreeMap::new();
//! let mut changes = system.changes("order/");
//! while let Some(change) = changes.next().await {
//!     match change {
//!         ActorChange::Resync => {
//!             // Replace the whole view, page by page; hints that arrive meanwhile wait in the stream.
//!             view.clear();
//!             let mut cursor = None;
//!             loop {
//!                 let page = system.list_actors_with_state("order/", cursor.as_ref(), 100).await?;
//!                 for actor in page.items {
//!                     view.insert(actor.address.clone(), actor);
//!                 }
//!                 match page.next {
//!                     Some(next) => cursor = Some(next),
//!                     None => break,
//!                 }
//!             }
//!         }
//!         ActorChange::Changed { address, .. } => match system.get_actor(&address).await? {
//!             Some(actor) => {
//!                 view.insert(address, actor);
//!             }
//!             None => {
//!                 view.remove(&address);
//!             }
//!         },
//!         _ => {}
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! **What is covered.** A committed create (by an operator or a turn, a child too), a turn's new state (also one equal
//! to the old), a pause or resume, a restart, and a removal by an operator delete, a turn's delete or a stop, with
//! every descendant. Each address once per transaction; a transaction with more than 100 addresses, or more than 7,998
//! bytes of them encoded, sends one `Resync` instead. Mailbox changes, leases, watches, node rows
//! and sweeps send nothing, and neither does a rolled-back, refused or fenced commit.
//!
//! **Scope.** The memory and SQLite stores report the writes through the same [`Store`](crate::Store) and its clones,
//! not those through a store opened separately on the same file, even in one process. PostgreSQL reports the writes of
//! every upgraded node on the same database and schema, through `NOTIFY` in the writing transaction. Old nodes and SQL
//! written by hand report nothing.
//!
//! **Best effort.** There is no history, no replay, no exactly-once delivery and no order across transactions. A slow
//! reader, a lost PostgreSQL connection and a commit whose outcome a SQLite store cannot know turn into `Resync`. A
//! host that must reconcile beyond this scope keeps its own periodic or explicit full refresh.

// Without both SQL backends, part of the collection and of the hub's source states is unused.
#![cfg_attr(not(all(feature = "sqlite", feature = "postgres", feature = "testing")), allow(dead_code))]

use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures::stream::{BoxStream, Stream, StreamExt};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::address::ActorAddress;

/// The most addresses one transaction reports; one more turns its hints into one [`ActorChange::Resync`].
pub(crate) const MAX_ADDRESSES: usize = 100;

/// The most bytes of one transaction's encoded hints: `@`, then the addresses separated by one space each (PostgreSQL
/// refuses a `NOTIFY` payload of 8000 bytes or more). One more byte turns its hints into one [`ActorChange::Resync`].
pub(crate) const MAX_PAYLOAD_BYTES: usize = 7_998;

/// The items a store's hub holds for its subscribers before a slow one lags and resyncs.
pub(crate) const HUB_CAPACITY: usize = 1024;

/// The hub items a stream skips (other prefixes, wake-ups) before it yields to the runtime in one poll.
const SKIPS_PER_POLL: usize = 64;

/// A hint of the change feed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActorChange {
    /// The actor at `address` may have changed: read it again. `None` from
    /// [`get_actor`](crate::ActorSystem::get_actor) means it is gone. The hint carries no state or version, because a
    /// version starts at 0 again when an address is created anew.
    #[non_exhaustive]
    Changed {
        /// The canonical address.
        address: ActorAddress,
    },
    /// Hints may have been missed: read the whole prefix again
    /// ([`list_actors_with_state`](crate::ActorSystem::list_actors_with_state)), then go on with the hints that follow.
    Resync,
}

impl ActorChange {
    /// A [`Self::Changed`] of `address`.
    pub(crate) fn changed(address: ActorAddress) -> Self {
        ActorChange::Changed { address }
    }
}

/// The stream of [`ActorChange`] hints of [`ActorSystem::changes`](crate::ActorSystem::changes). It owns what it needs
/// (it borrows no system), is `Send` and `Unpin`, and its `next` is cancel-safe: a dropped `next` loses no hint. It has
/// no error items and ends only when the store is closed.
pub struct ActorChanges {
    inner: BoxStream<'static, ActorChange>,
}

impl ActorChanges {
    /// A stream of `hub`'s hints under the literal `prefix`; subscribed before it is returned.
    pub(crate) fn new(hub: Arc<ChangeHub>, prefix: &str) -> Self {
        let rx = hub.subscribe().map(|(rx, _)| rx);
        let feed = Feed { hub, prefix: prefix.to_owned(), rx, step: Step::Resync, queue: VecDeque::new() };
        let inner = futures::stream::unfold(feed, |mut feed| async move { feed.next().await.map(|c| (c, feed)) });
        Self { inner: inner.fuse().boxed() }
    }
}

impl fmt::Debug for ActorChanges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorChanges").finish_non_exhaustive()
    }
}

impl Stream for ActorChanges {
    type Item = ActorChange;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ActorChange>> {
        self.inner.poll_next_unpin(cx)
    }
}

// ---- collection ----

/// The hints of one transaction: a set of addresses, or a whole batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChangeBatch {
    /// These addresses, each once, in address order.
    Addresses(Arc<[ActorAddress]>),
    /// Too many to name, or unknown: every subscriber rereads.
    Resync,
}

impl ChangeBatch {
    /// The PostgreSQL payload: `@` alone for a resync, else `@` and the addresses separated by single spaces.
    pub(crate) fn encode(&self) -> String {
        match self {
            ChangeBatch::Resync => "@".to_owned(),
            ChangeBatch::Addresses(addresses) => {
                let mut payload = String::from("@");
                for (i, address) in addresses.iter().enumerate() {
                    if i > 0 {
                        payload.push(' ');
                    }
                    payload.push_str(address.as_str());
                }
                payload
            }
        }
    }

    /// The batch of a PostgreSQL payload, or `None` for one that is not a feed payload (it does not start with `@`).
    /// A malformed feed payload (an empty or non-canonical token, a leading, double or trailing space) is a resync, as
    /// a whole: no valid part of it is reported.
    pub(crate) fn parse(payload: &str) -> Option<Self> {
        let rest = payload.strip_prefix('@')?;
        if rest.is_empty() {
            return Some(ChangeBatch::Resync);
        }
        let mut addresses = Vec::new();
        for token in rest.split(' ') {
            match ActorAddress::parse(token) {
                Ok(address) if address.as_str() == token => addresses.push(address),
                _ => return Some(ChangeBatch::Resync),
            }
        }
        Some(ChangeBatch::Addresses(addresses.into()))
    }
}

/// Collects the addresses one transaction changes, each once, up to [`MAX_ADDRESSES`] and [`MAX_PAYLOAD_BYTES`]; past
/// either bound, or after [`Self::overflow`], it holds a resync for good.
#[derive(Debug, Default)]
pub(crate) struct ChangeCollector {
    addresses: BTreeSet<ActorAddress>,
    /// The encoded size of `addresses`: `@` and the addresses with a space between each two.
    bytes: usize,
    overflow: bool,
}

impl ChangeCollector {
    /// Adds `address`, once.
    pub(crate) fn add(&mut self, address: &ActorAddress) {
        if self.overflow || self.addresses.contains(address) {
            return;
        }
        let bytes = if self.addresses.is_empty() { 1 } else { self.bytes + 1 } + address.as_str().len();
        if self.addresses.len() + 1 > MAX_ADDRESSES || bytes > MAX_PAYLOAD_BYTES {
            self.overflow();
            return;
        }
        self.addresses.insert(address.clone());
        self.bytes = bytes;
    }

    /// Adds every address of `addresses`.
    pub(crate) fn add_all<'a>(&mut self, addresses: impl IntoIterator<Item = &'a ActorAddress>) {
        for address in addresses {
            self.add(address);
        }
    }

    /// Turns the transaction's hints into one resync, for good: its addresses are too many, or unknown.
    pub(crate) fn overflow(&mut self) {
        self.overflow = true;
        self.addresses = BTreeSet::new();
        self.bytes = 0;
    }

    /// True when the transaction changed nothing the feed reports.
    pub(crate) fn is_empty(&self) -> bool {
        !self.overflow && self.addresses.is_empty()
    }

    /// The batch to publish, or `None` when there is nothing to report; leaves the collector empty.
    pub(crate) fn take(&mut self) -> Option<ChangeBatch> {
        let collected = std::mem::take(self);
        if collected.overflow {
            Some(ChangeBatch::Resync)
        } else if collected.addresses.is_empty() {
            None
        } else {
            Some(ChangeBatch::Addresses(collected.addresses.into_iter().collect()))
        }
    }
}

// ---- the hub ----

/// Whether a hub's source delivers every hint: `Up`, waiting to (`Down`, as PostgreSQL before its `LISTEN`), or
/// never again (`Closed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceState {
    Up,
    Down,
    Closed,
}

/// An item of a hub, in source order.
#[derive(Debug, Clone)]
pub(crate) enum HubItem {
    /// The source is live again: every subscriber resyncs, and wake-ups wake.
    Up,
    /// The source lost its connection: hints may be missed until the next `Up`.
    Down,
    /// The addresses of one transaction.
    Changes(Arc<[ActorAddress]>),
    /// A transaction whose addresses are not named.
    Resync,
    /// A PostgreSQL wake-up of the type with this name.
    WakeUp(Arc<str>),
}

/// The change source of one store, shared by its clones: a bounded broadcast of [`HubItem`]s and the source state. One
/// short lock serialises the changes of the state, publication, and subscribing with a reading of the state; it is
/// never held across an `await`.
pub(crate) struct ChangeHub {
    inner: Mutex<HubInner>,
    closed: AtomicBool,
}

struct HubInner {
    state: SourceState,
    /// `None` once closed: dropping the sender wakes every waiting receiver.
    sender: Option<broadcast::Sender<HubItem>>,
}

impl fmt::Debug for ChangeHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChangeHub").field("state", &self.state()).finish()
    }
}

impl ChangeHub {
    /// A hub whose source starts in `state` (`Up` for the memory and SQLite stores, `Down` for PostgreSQL).
    pub(crate) fn new(state: SourceState) -> Self {
        let sender = broadcast::channel(HUB_CAPACITY).0;
        Self { inner: Mutex::new(HubInner { state, sender: Some(sender) }), closed: AtomicBool::new(false) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The source state now.
    pub(crate) fn state(&self) -> SourceState {
        self.lock().state
    }

    /// True once [`Self::close`] ran.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// A new receiver and the state at the moment it was taken, atomically; `None` once closed.
    pub(crate) fn subscribe(&self) -> Option<(broadcast::Receiver<HubItem>, SourceState)> {
        let inner = self.lock();
        inner.sender.as_ref().map(|sender| (sender.subscribe(), inner.state))
    }

    fn send(&self, item: HubItem) {
        if let Some(sender) = &self.lock().sender {
            let _ = sender.send(item); // no receiver: nobody to tell
        }
    }

    /// Publishes the batch of a committed transaction.
    pub(crate) fn publish(&self, batch: ChangeBatch) {
        self.send(match batch {
            ChangeBatch::Addresses(addresses) => HubItem::Changes(addresses),
            ChangeBatch::Resync => HubItem::Resync,
        });
    }

    /// Publishes what `changes` collected, if anything.
    pub(crate) fn publish_collected(&self, changes: &mut ChangeCollector) {
        if let Some(batch) = changes.take() {
            self.publish(batch);
        }
    }

    /// Publishes a PostgreSQL notification's payload: a feed payload (`@…`) as its batch, any other non-empty one as
    /// a wake-up of that type.
    pub(crate) fn payload(&self, payload: &str) {
        match ChangeBatch::parse(payload) {
            Some(batch) => self.publish(batch),
            None if !payload.is_empty() => self.send(HubItem::WakeUp(payload.into())),
            None => {}
        }
    }

    /// The source is live: every subscriber resyncs against it. Ignored once closed.
    pub(crate) fn up(&self) {
        self.set(SourceState::Up, HubItem::Up);
    }

    /// The source lost its connection. Ignored once closed.
    pub(crate) fn down(&self) {
        self.set(SourceState::Down, HubItem::Down);
    }

    fn set(&self, state: SourceState, item: HubItem) {
        let mut inner = self.lock();
        if let Some(sender) = &inner.sender {
            let _ = sender.send(item);
            inner.state = state;
        }
    }

    /// Ends the hub for good: every stream ends at its next poll, without its backlog.
    pub(crate) fn close(&self) {
        let mut inner = self.lock();
        inner.state = SourceState::Closed;
        inner.sender = None;
        self.closed.store(true, Ordering::SeqCst);
    }
}

// ---- one subscription ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Take a fresh receiver and read the state with it: yield `Resync` when the source is up.
    Resync,
    /// The source is down: wait for it to come up, skipping hints (the resync covers them).
    WaitUp,
    /// Yield the hints under the prefix.
    Live,
}

/// The state of one [`ActorChanges`].
struct Feed {
    hub: Arc<ChangeHub>,
    prefix: String,
    rx: Option<broadcast::Receiver<HubItem>>,
    step: Step,
    /// Hints of a received batch not yielded yet.
    queue: VecDeque<ActorAddress>,
}

impl Feed {
    async fn next(&mut self) -> Option<ActorChange> {
        let mut skipped = 0;
        loop {
            if self.hub.is_closed() {
                self.end();
                return None;
            }
            if self.step == Step::Resync {
                let Some((rx, state)) = self.hub.subscribe() else {
                    self.end();
                    return None;
                };
                self.rx = Some(rx);
                self.queue.clear();
                match state {
                    SourceState::Up => {
                        self.step = Step::Live;
                        return Some(ActorChange::Resync);
                    }
                    SourceState::Down => self.step = Step::WaitUp,
                    SourceState::Closed => {
                        self.end();
                        return None;
                    }
                }
            }
            if let Some(address) = self.queue.pop_front() {
                return Some(ActorChange::changed(address));
            }
            let rx = self.rx.as_mut()?;
            match rx.recv().await {
                Ok(HubItem::Changes(addresses)) if self.step == Step::Live => {
                    let prefix = self.prefix.as_str();
                    self.queue.extend(addresses.iter().filter(|a| a.has_prefix(prefix)).cloned());
                    if self.queue.is_empty() {
                        skipped += 1;
                    }
                }
                Ok(HubItem::Up | HubItem::Resync) | Err(RecvError::Lagged(_)) => self.step = Step::Resync,
                Ok(HubItem::Down) => self.step = Step::WaitUp,
                Ok(HubItem::Changes(_) | HubItem::WakeUp(_)) => skipped += 1,
                Err(RecvError::Closed) => {
                    self.end();
                    return None;
                }
            }
            if skipped >= SKIPS_PER_POLL {
                skipped = 0;
                tokio::task::yield_now().await;
            }
        }
    }

    fn end(&mut self) {
        self.rx = None;
        self.queue.clear();
    }
}
