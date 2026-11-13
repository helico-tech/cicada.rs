//! Turn observers: code that watches the turns of a node start and end, without a say in them (ADR 0018).
//!
//! A node calls [`TurnObserver::turn_started`] of each observer when it starts a turn: its message is claimed, and
//! nothing is decoded or run yet. The value that call returns, an [`ObservedTurn`], is told the outcome with
//! [`ObservedTurn::turn_ended`] once the commit of the turn succeeded. A value that is dropped without that call means
//! that the node saw no commit of the turn succeed: its lease was lost or ran out, its node stopped and gave the lease
//! back, its commit could not get through, or its actor was deleted while it ran.
//!
//! | Turn | `turn_started` | `turn_ended` |
//! | --- | --- | --- |
//! | Handler returns | yes | `done`, or the `failed` the node committed in place of a rejected commit |
//! | Handler fails or exceeds its maximum duration | yes | `failed`, or a reschedule when the message's retry policy retries it |
//! | Message or state cannot be decoded | yes | `failed` |
//! | An operator cancels the turn | yes | `cancelled` |
//! | The claim limit fails the message | after its commit | `failed`, right after the start |
//! | No commit that the node saw succeed | yes | – (the value is dropped) |
//! | The answer of the claim arrives after its lease deadline | – | – (nothing runs; the claim is given back) |
//!
//! An observer cannot refuse a turn and cannot change what it commits, except that it may add headers to the messages
//! the turn sends ([`TurnStart::add_send_header`], or [`TurnStart::add_header_for`] to pick timers or the other
//! sends). Its methods are plain functions: they run on a thread of the
//! runtime, in the turn, and must not block. A panic in an observer is caught and logged, and the turn goes on as if
//! that observer were not there. The observers of a node are called in the order they were added, at the start and at
//! the end.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use crate::actor::panic_message;
use crate::address::{ActorAddress, TypeName};
use crate::runtime::limits;
use crate::store::{Message, TurnOutcome};
use crate::time::Timestamp;

/// Watches the turns of a node. It has no say in them. Add one with
/// [`ActorSystemBuilder::observe`](crate::runtime::ActorSystemBuilder::observe); an `Arc` of an observer is an
/// observer too, so a test can keep a handle. Its methods run on a thread of the runtime, in the turn: one that blocks
/// holds up that thread and the turn, so it keeps books and hands slow work to a task of its own.
///
/// ```
/// use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
///
/// /// Prints how each turn ends.
/// struct Audit;
///
/// struct AuditedTurn {
///     message: cicada::store::MessageId,
/// }
///
/// impl TurnObserver for Audit {
///     fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
///         turn.add_send_header("audit", "on");
///         Box::new(AuditedTurn { message: turn.message().id.clone() })
///     }
/// }
///
/// impl ObservedTurn for AuditedTurn {
///     fn turn_ended(self: Box<Self>, end: &TurnEnd<'_>) {
///         println!("{} -> {:?}, {} sent", self.message, end.outcome(), end.sent().len());
///     }
/// }
/// ```
pub trait TurnObserver: Send + Sync + 'static {
    /// A turn starts: its message is claimed, nothing is loaded or run yet. The returned value is told the outcome
    /// when the commit of the turn succeeded, or dropped without it; return `Box::new(())` to be told nothing.
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn>;
}

impl<O: TurnObserver + ?Sized> TurnObserver for Arc<O> {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        (**self).turn_started(turn)
    }
}

/// One turn, as its observer holds it. Dropped without [`Self::turn_ended`]: the node saw no commit of the turn succeed.
/// An observer that cares keeps a flag and implements `Drop`.
pub trait ObservedTurn: Send {
    /// The commit of the turn succeeded, with what [`TurnEnd`] says was committed.
    fn turn_ended(self: Box<Self>, end: &TurnEnd<'_>);
}

/// Nothing to be told: for an observer that only adds headers.
impl ObservedTurn for () {
    fn turn_ended(self: Box<Self>, _end: &TurnEnd<'_>) {}
}

/// The sends of a turn that a header of an observer goes on ([`TurnStart::add_header_for`]).
///
/// A timer is a send to the turn's own actor with a delivery of [`Delivery::At`](crate::store::Delivery::At) or
/// [`Delivery::After`](crate::store::Delivery::After), whatever the time asked for: a zero delay or a past instant
/// counts, though the store makes such a message due at once. The send method does not matter, and neither does the
/// actor's type. Every other send of the turn is one of the others, a delayed send to another actor too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sends {
    /// Every send of the turn: timers and the others.
    All,
    /// The sends to the turn's own actor with a later delivery: `At` or `After`.
    Timers,
    /// Every send that is not a timer.
    Others,
}

/// The headers observers added for the sends of a turn: one map for timers, one for the other sends.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SendHeaders {
    timers: BTreeMap<String, String>,
    others: BTreeMap<String, String>,
}

impl SendHeaders {
    /// The headers for a timer when `timer`, else those for another send.
    pub(crate) fn of(&self, timer: bool) -> &BTreeMap<String, String> {
        if timer { &self.timers } else { &self.others }
    }

    fn insert(&mut self, sends: Sends, name: String, value: String) {
        match sends {
            Sends::All => {
                self.timers.insert(name.clone(), value.clone());
                self.others.insert(name, value);
            }
            Sends::Timers => {
                self.timers.insert(name, value);
            }
            Sends::Others => {
                self.others.insert(name, value);
            }
        }
    }

    fn extend(&mut self, later: SendHeaders) {
        self.timers.extend(later.timers);
        self.others.extend(later.others);
    }
}

/// A turn that starts, as [`TurnObserver::turn_started`] sees it.
#[derive(Debug)]
pub struct TurnStart<'a> {
    actor: &'a ActorAddress,
    message: &'a Message,
    claimed_at: Timestamp,
    headers: SendHeaders,
}

impl<'a> TurnStart<'a> {
    pub(crate) fn new(actor: &'a ActorAddress, message: &'a Message, claimed_at: Timestamp) -> Self {
        Self { actor, message, claimed_at, headers: SendHeaders::default() }
    }

    /// The actor whose turn this is.
    pub fn actor(&self) -> &ActorAddress {
        self.actor
    }

    /// The actor's type.
    pub fn type_name(&self) -> &TypeName {
        self.actor.type_name()
    }

    /// The message, as claimed: its `attempt` counts this claim.
    pub fn message(&self) -> &Message {
        self.message
    }

    /// Store time of the claim.
    pub fn claimed_at(&self) -> Timestamp {
        self.claimed_at
    }

    /// Adds a header that every message the handler of this turn sends will carry, such as the turn's trace context.
    /// A header that a send sets itself keeps the send's value, and of two observers that add one name the one added
    /// later wins.
    ///
    /// A header never fails a send. One that the limits of a message refuse (a name of more than 200 or a value of more
    /// than 4,000 characters, or a NUL character) is left out here, with a warning. A send takes its own headers first
    /// and then those of the observers, in the order of their names, as long as the message stays within 64 headers;
    /// the ones that do not fit are left out of that message, with a warning.
    ///
    /// The same as [`Self::add_header_for`] with [`Sends::All`].
    pub fn add_send_header(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.add_header_for(Sends::All, name, value);
    }

    /// Adds a header for the sends of this turn that `sends` picks: all of them, the timers, or the others. The rules
    /// of [`Self::add_send_header`] hold for each kind on its own: of two calls for one name the later wins in the
    /// kinds it picks, and of two observers the one added later. A refused header is left out, with a warning, and
    /// keeps the value an earlier call gave that name.
    ///
    /// ```
    /// use cicada::observer::{Sends, TurnObserver, ObservedTurn, TurnStart};
    ///
    /// /// Marks the timers of each turn, and keeps a chain id on the other sends.
    /// struct Marks;
    ///
    /// impl TurnObserver for Marks {
    ///     fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
    ///         turn.add_header_for(Sends::Timers, "timer", "yes");
    ///         turn.add_header_for(Sends::Others, "chain", turn.message().id.to_string());
    ///         Box::new(())
    ///     }
    /// }
    /// ```
    pub fn add_header_for(&mut self, sends: Sends, name: impl Into<String>, value: impl Into<String>) {
        let (name, value) = (name.into(), value.into());
        match limits::check_header(&name, &value) {
            Ok(()) => self.headers.insert(sends, name, value),
            Err(e) => tracing::warn!(
                actor = %self.actor,
                message = %self.message.id,
                "A header of an observer is left out of the sends of the turn: {e}"
            ),
        }
    }
}

/// How a turn ended, as [`ObservedTurn::turn_ended`] sees it: what its commit wrote.
#[derive(Debug)]
pub struct TurnEnd<'a> {
    outcome: &'a TurnOutcome,
    sent: &'a [Message],
}

impl<'a> TurnEnd<'a> {
    pub(crate) fn new(outcome: &'a TurnOutcome, sent: &'a [Message]) -> Self {
        Self { outcome, sent }
    }

    /// The committed outcome: `done`, `failed` with its error, a reschedule with its error and delivery, or
    /// `cancelled`. When the store rejected or refused the turn's commit, the `failed` the node committed in its place.
    pub fn outcome(&self) -> &TurnOutcome {
        self.outcome
    }

    /// The messages the commit sent, with their store-assigned `seq` and `deliver_at`. Notifications of watches are
    /// not among them: the store writes those.
    pub fn sent(&self) -> &[Message] {
        self.sent
    }
}

/// The observers of a node, in the order they were added.
#[derive(Clone, Default)]
pub(crate) struct Observers(Vec<Arc<dyn TurnObserver>>);

impl Observers {
    pub(crate) fn push(&mut self, observer: Arc<dyn TurnObserver>) {
        self.0.push(observer);
    }

    /// Calls `turn_started` of every observer, in order. A call that panics is logged and counts as not made: its
    /// headers are not used and it has no value.
    pub(crate) fn start(&self, actor: &ActorAddress, message: &Message, claimed_at: Timestamp) -> Observed {
        let mut observed = Observed::default();
        for observer in &self.0 {
            let mut start = TurnStart::new(actor, message, claimed_at);
            match catch_unwind(AssertUnwindSafe(|| observer.turn_started(&mut start))) {
                Ok(turn) => {
                    observed.headers.extend(start.headers);
                    observed.turns.push(turn);
                }
                Err(panic) => tracing::warn!(
                    actor = %actor,
                    message = %message.id,
                    "An observer panicked when the turn started; it is left out of this turn: {}",
                    panic_message(&*panic)
                ),
            }
        }
        observed
    }
}

/// What the observers of one turn returned: their values, in order, and the headers for the turn's sends. Dropped
/// without [`Self::ended`], it drops each value, catching a panic of each.
#[derive(Default)]
pub(crate) struct Observed {
    turns: Vec<Box<dyn ObservedTurn>>,
    headers: SendHeaders,
}

impl Observed {
    /// The headers the observers added for the sends of the turn, per kind; of two with one name, the later observer's.
    pub(crate) fn headers(&self) -> &SendHeaders {
        &self.headers
    }

    /// Tells every value, in order, that the commit succeeded with `outcome` and `sent`, catching a panic of each.
    pub(crate) fn ended(mut self, outcome: &TurnOutcome, sent: &[Message]) {
        let end = TurnEnd::new(outcome, sent);
        for turn in std::mem::take(&mut self.turns) {
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| turn.turn_ended(&end))) {
                tracing::warn!("An observer panicked when the turn ended: {}", panic_message(&*panic));
            }
        }
    }
}

impl Drop for Observed {
    fn drop(&mut self) {
        for turn in std::mem::take(&mut self.turns) {
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(turn))) {
                tracing::warn!("An observer panicked when its turn was dropped: {}", panic_message(&*panic));
            }
        }
    }
}
