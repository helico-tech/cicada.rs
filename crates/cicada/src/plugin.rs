//! Turn plugins: three hooks that observe a turn. A plugin never changes what a turn commits, with one exception: in
//! `on_claim` it may add headers to the messages the turn sends (trace context, see [`ClaimContext::send_headers`]).
//! The one thing it can decide is to refuse a turn in `on_claim`.
//!
//! | Turn | `on_claim` | handler | `on_handler_error` | `on_outcome` |
//! | --- | --- | --- | --- | --- |
//! | Handler returns | yes | runs | – | `done` (or the rejection) |
//! | Handler fails | yes | runs | yes | `failed`, or rescheduled when its retry policy retries it |
//! | Turn exceeds its maximum duration | yes | cancelled | yes, the failure is a timeout | `failed` (`timeout`), or rescheduled |
//! | Message undecodable | yes ([`TurnInfo::unhandled`] set) | – | – | `failed`, never retried |
//! | State or own address undecodable at load | – | – | – | `failed` |
//! | `on_claim` fails | up to the failing hook | – | – | `failed` |
//! | Lease lost before commit | maybe | maybe | maybe | – (no outcome; the turn is re-run) |
//!
//! Plugins are installed on a node and run for every type it hosts, in installation order.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::address::{ActorAddress, TypeName};
use crate::store::{Message, TurnOutcome};
use crate::time::Timestamp;
use crate::turn::HandlerFailure;

/// What a hook returns: `Err` fails the turn (`on_claim`) or is logged and ignored (`on_handler_error`,
/// `on_outcome`).
pub type HookResult = Result<(), anyhow::Error>;

/// A turn plugin: a name and up to three hooks. Install one on a node, for every type it hosts. Its configuration is
/// the plugin value itself. An `Arc` of a plugin is a plugin too, so one value can be shared.
///
/// ```
/// use cicada::plugin::{HookResult, OutcomeContext, Plugin};
///
/// struct Audit {
///     prefix: String,
/// }
///
/// #[async_trait::async_trait]
/// impl Plugin for Audit {
///     fn name(&self) -> &str {
///         "Audit"
///     }
///
///     async fn on_outcome(&self, ctx: &OutcomeContext<'_>) -> HookResult {
///         println!("{}: {} -> {:?}", self.prefix, ctx.info().message().id, ctx.outcome());
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// Unique name; a plugin is installed at most once per node.
    fn name(&self) -> &str;

    /// After load, before the handler: observe the turn, put values in its attributes, and add headers to its sends
    /// ([`ClaimContext::send_headers`]). An error refuses the turn:
    /// the handler does not run and the message ends `failed` (no `on_handler_error`).
    async fn on_claim(&self, _ctx: &ClaimContext<'_>) -> HookResult {
        Ok(())
    }

    /// When the handler fails or exceeds its maximum turn duration: observe the failure and whether the message's
    /// retry policy retries it. An error is logged and ignored.
    async fn on_handler_error(&self, _ctx: &HandlerErrorContext<'_>) -> HookResult {
        Ok(())
    }

    /// After a successful commit: observe the final outcome and the messages it enqueued. Not called when the lease
    /// was lost and nothing was committed. An error is logged and ignored.
    async fn on_outcome(&self, _ctx: &OutcomeContext<'_>) -> HookResult {
        Ok(())
    }
}

#[async_trait]
impl<P: Plugin + ?Sized> Plugin for Arc<P> {
    fn name(&self) -> &str {
        (**self).name()
    }

    async fn on_claim(&self, ctx: &ClaimContext<'_>) -> HookResult {
        (**self).on_claim(ctx).await
    }

    async fn on_handler_error(&self, ctx: &HandlerErrorContext<'_>) -> HookResult {
        (**self).on_handler_error(ctx).await
    }

    async fn on_outcome(&self, ctx: &OutcomeContext<'_>) -> HookResult {
        (**self).on_outcome(ctx).await
    }
}

/// A typed key-value bag that lives for one turn, shared by the handler and every hook of the turn. Values are keyed
/// by their type: define a newtype per purpose. Cloning yields another handle to the same bag.
#[derive(Clone, Default)]
pub struct Attributes(Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>>);

impl Attributes {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<TypeId, Box<dyn Any + Send + Sync>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets the value of type `T`, returning the previous one.
    pub fn insert<T: Clone + Send + Sync + 'static>(&self, value: T) -> Option<T> {
        self.map().insert(TypeId::of::<T>(), Box::new(value)).and_then(|old| old.downcast::<T>().ok().map(|b| *b))
    }

    /// A copy of the value of type `T`.
    pub fn get<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.map().get(&TypeId::of::<T>()).and_then(|v| v.downcast_ref::<T>()).cloned()
    }

    /// The value of type `T`, computing and storing it with `f` when absent.
    pub fn get_or_insert_with<T: Clone + Send + Sync + 'static>(&self, f: impl FnOnce() -> T) -> T {
        let mut map = self.map();
        let entry = map.entry(TypeId::of::<T>()).or_insert_with(|| Box::new(f()));
        entry.downcast_ref::<T>().cloned().expect("attribute keyed by its own type")
    }
}

impl fmt::Debug for Attributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Attributes({} values)", self.map().len())
    }
}

/// What every hook can see of the turn.
#[derive(Debug)]
pub struct TurnInfo {
    pub(crate) self_address: ActorAddress,
    pub(crate) message: Message,
    pub(crate) now: Timestamp,
    pub(crate) attributes: Attributes,
    pub(crate) send_headers: Mutex<BTreeMap<String, String>>,
    pub(crate) unhandled: std::sync::OnceLock<String>,
    pub(crate) claim_sent: tokio::time::Instant,
}

impl TurnInfo {
    /// The instance whose turn this is.
    pub fn self_address(&self) -> &ActorAddress {
        &self.self_address
    }

    /// The instance's type.
    pub fn type_name(&self) -> &TypeName {
        self.self_address.type_name()
    }

    /// The message being handled, as claimed: its `attempt` already counts this claim.
    pub fn message(&self) -> &Message {
        &self.message
    }

    /// Store time at the claim.
    pub fn now(&self) -> Timestamp {
        self.now
    }

    /// The turn's attributes, shared with the handler and every later hook of this turn.
    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Why the message could not be decoded (an unknown type name or a payload that does not fit), when no handler
    /// runs and the message ends `failed`.
    pub fn unhandled(&self) -> Option<&str> {
        self.unhandled.get().map(String::as_str)
    }

    /// `now` plus the local time that passed since the claim was sent.
    pub fn current_time(&self) -> Timestamp {
        let elapsed = self.claim_sent.elapsed();
        crate::time::add(self.now, elapsed).unwrap_or(self.now)
    }
}

/// Context of [`Plugin::on_claim`].
pub struct ClaimContext<'a> {
    pub(crate) info: &'a TurnInfo,
}

impl ClaimContext<'_> {
    /// The turn.
    pub fn info(&self) -> &TurnInfo {
        self.info
    }

    /// Adds headers that every message this turn sends will carry, such as the turn's trace context. A send that
    /// sets a header of the same name itself keeps its own value.
    pub fn send_headers(&self, headers: impl IntoIterator<Item = (String, String)>) {
        self.info.send_headers.lock().unwrap_or_else(|e| e.into_inner()).extend(headers);
    }
}

/// Context of [`Plugin::on_handler_error`]. The runtime has already decided, from the message's retry policy, whether
/// the message is retried.
pub struct HandlerErrorContext<'a> {
    pub(crate) info: &'a TurnInfo,
    pub(crate) error: &'a HandlerFailure,
    pub(crate) retry_delay: Option<Duration>,
}

impl HandlerErrorContext<'_> {
    /// The turn.
    pub fn info(&self) -> &TurnInfo {
        self.info
    }

    /// How the handler failed.
    pub fn error(&self) -> &HandlerFailure {
        self.error
    }

    /// True when the message goes back to pending for another attempt; false when it ends `failed`.
    pub fn will_retry(&self) -> bool {
        self.retry_delay.is_some()
    }

    /// The backoff before the next attempt; `None` when the message is not retried.
    pub fn retry_delay(&self) -> Option<Duration> {
        self.retry_delay
    }
}

/// Context of [`Plugin::on_outcome`].
pub struct OutcomeContext<'a> {
    pub(crate) info: &'a TurnInfo,
    pub(crate) outcome: &'a TurnOutcome,
    pub(crate) sent: &'a [Message],
}

impl OutcomeContext<'_> {
    /// The turn.
    pub fn info(&self) -> &TurnInfo {
        self.info
    }

    /// The committed outcome: a final outcome, or a reschedule when the message went back to pending.
    pub fn outcome(&self) -> &TurnOutcome {
        self.outcome
    }

    /// The messages the commit enqueued, with their store-assigned `seq` and `deliver_at`.
    pub fn sent(&self) -> &[Message] {
        self.sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct TraceId(String);

    #[test]
    fn attributes_are_shared_and_typed() {
        let a = Attributes::default();
        let b = a.clone();
        assert!(a.insert(TraceId("t1".into())).is_none());
        assert_eq!(b.get::<TraceId>(), Some(TraceId("t1".into())));
        assert_eq!(b.get_or_insert_with(|| 5u32), 5);
        assert_eq!(a.insert(TraceId("t2".into())), Some(TraceId("t1".into())));
        assert_eq!(a.get::<u32>(), Some(5));
    }
}
