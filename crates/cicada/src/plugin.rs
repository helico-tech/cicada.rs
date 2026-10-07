//! Turn plugins: four hooks that observe a turn and may add effects to its commit.
//!
//! | Turn | `on_claim` | handler | `on_handler_error` | `before_commit` | `on_outcome` |
//! | --- | --- | --- | --- | --- | --- |
//! | Handler returns | yes | runs | – | yes | `done` (or the rejection) |
//! | Handler fails | yes | runs | yes | – | `failed`, rescheduled when its retry policy retries it, or what the error hooks decided |
//! | Turn exceeds its maximum duration | yes | cancelled | yes, the failure is a timeout | – | `failed` (`timeout`), rescheduled, or what the error hooks decided |
//! | Payload undecodable, unhandled policy `Fail`/`Drop` | yes ([`TurnInfo::unhandled`] set) | – | – | – | `failed` / `done` |
//! | Payload undecodable, custom policy | yes | the policy runs like a handler | if it fails or times out | if it returns | as for a handler |
//! | State or own address undecodable at load | – | – | – | – | `failed` |
//! | `on_claim` or `before_commit` fails | up to the failing hook | – | – | up to the failing hook | `failed` |
//! | Lease lost before commit | maybe | maybe | maybe | maybe | – (no outcome; the turn is re-run) |
//!
//! Effects a hook adds join the turn's single commit, so the guarantees hold with or without plugins. Node plugins run
//! before type plugins, each in installation order. A plugin installed on both the node and a type runs once for that
//! type, with the type's installation (matched by [`Plugin::name`]).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::address::{ActorAddress, UriTemplate};
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::runtime::effects::RawSender;
use crate::store::{Delivery, Message, MessageId, TurnOutcome};
use crate::time::Timestamp;
use crate::turn::{HandlerFailure, TurnError};

/// What a hook returns: `Err` fails the turn (`on_claim`, `before_commit`) or is logged and ignored
/// (`on_handler_error`, `on_outcome`).
pub type HookResult = Result<(), anyhow::Error>;

/// A turn plugin: a name and up to four hooks. Install one on a node (for every type it hosts) or on one actor type.
/// Its configuration is the plugin value itself.
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
    /// Unique name; a plugin is installed at most once per node and once per type.
    fn name(&self) -> &str;

    /// After load, before the handler (or the custom unhandled policy): observe the turn and put values in its
    /// attributes. An error fails the turn with outcome `failed` (no `on_handler_error`, no effects).
    async fn on_claim(&self, _ctx: &mut ClaimContext<'_>) -> HookResult {
        Ok(())
    }

    /// When the handler fails or exceeds its maximum turn duration. The handler's buffered effects are already
    /// discarded. The runtime has already decided, from the message's retry policy, whether it is retried
    /// ([`HandlerErrorContext::will_retry`]); the hook sees that decision, may add effects of its own, and may replace
    /// the failure with a final outcome, which also cancels the retry. An error is logged and ignored, and the effects
    /// and outcome change the hook made are rolled back.
    async fn on_handler_error(&self, _ctx: &mut HandlerErrorContext<'_>) -> HookResult {
        Ok(())
    }

    /// After the handler (or a custom unhandled policy) returns, before the commit: observe or add effects. An error
    /// fails the turn with outcome `failed` and discards all of its effects.
    async fn before_commit(&self, _ctx: &mut BeforeCommitContext<'_>) -> HookResult {
        Ok(())
    }

    /// After a successful commit: observe the final outcome and the messages it enqueued. Not called when the lease
    /// was lost and nothing was committed. An error is logged and ignored.
    async fn on_outcome(&self, _ctx: &OutcomeContext<'_>) -> HookResult {
        Ok(())
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

    /// True when a value of type `T` is set.
    pub fn contains<T: 'static>(&self) -> bool {
        self.map().contains_key(&TypeId::of::<T>())
    }

    /// Removes and returns the value of type `T`.
    pub fn remove<T: Send + Sync + 'static>(&self) -> Option<T> {
        self.map().remove(&TypeId::of::<T>()).and_then(|old| old.downcast::<T>().ok().map(|b| *b))
    }

    /// The value of type `T`, computing and storing it with `f` when absent.
    pub fn get_or_insert_with<T: Clone + Send + Sync + 'static>(&self, f: impl FnOnce() -> T) -> T {
        let mut map = self.map();
        let entry = map.entry(TypeId::of::<T>()).or_insert_with(|| Box::new(f()));
        entry.downcast_ref::<T>().cloned().expect("attribute keyed by its own type")
    }

    /// Runs `f` with mutable access to the value of type `T`, if set.
    pub fn with_mut<T: Send + Sync + 'static, R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.map().get_mut(&TypeId::of::<T>()).and_then(|v| v.downcast_mut::<T>()).map(f)
    }
}

impl fmt::Debug for Attributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Attributes({} values)", self.map().len())
    }
}

/// Message headers reserved for the runtime and plugins: every name starting with `cicada-` (case-insensitive).
/// Hooks may put them on the messages they send; a send from outside an actor or from a handler that carries one is
/// refused, so that callers cannot forge a plugin's bookkeeping.
pub mod reserved_headers {
    /// The reserved prefix of header names.
    pub const PREFIX: &str = "cicada-";

    /// True when `name` is reserved.
    pub fn is_reserved(name: &str) -> bool {
        name.len() >= PREFIX.len() && name[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
    }
}

/// What every hook can see of the turn.
#[derive(Debug)]
pub struct TurnInfo {
    pub(crate) self_address: ActorAddress,
    pub(crate) template: UriTemplate,
    pub(crate) message: Message,
    pub(crate) now: Timestamp,
    pub(crate) attributes: Attributes,
    pub(crate) unhandled: std::sync::OnceLock<String>,
    pub(crate) claim_sent: tokio::time::Instant,
}

impl TurnInfo {
    /// The instance whose turn this is.
    pub fn self_address(&self) -> &ActorAddress {
        &self.self_address
    }

    /// The instance's type.
    pub fn template(&self) -> &UriTemplate {
        &self.template
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

    /// Why the payload could not be decoded, when the unhandled policy runs instead of a handler.
    pub fn unhandled(&self) -> Option<&str> {
        self.unhandled.get().map(String::as_str)
    }

    /// `now` plus the local time that passed since the claim was sent.
    pub fn current_time(&self) -> Timestamp {
        let elapsed = self.claim_sent.elapsed();
        crate::time::add(self.now, elapsed).unwrap_or(self.now)
    }
}

/// How a hook sends a message: like a raw send of a handler, but reserved headers are allowed and the id may be set.
#[derive(Debug, Clone, Default)]
pub struct HookSend {
    /// When it becomes due.
    pub delivery: Delivery,
    /// Opaque string passed through.
    pub correlation_id: Option<String>,
    /// Headers; reserved `cicada-` names are allowed for hooks.
    pub headers: std::collections::BTreeMap<String, String>,
    /// The id; a fresh one when `None`.
    pub id: Option<MessageId>,
    /// The new message's retry policy.
    pub retry: Option<RetryPolicy>,
    /// The new message's retention policy.
    pub retention: Option<Retention>,
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
}

/// Context of [`Plugin::on_handler_error`].
///
/// Order of decisions: the runtime first decides retry-or-fail from the message's retry policy; then every error hook
/// runs and sees that decision as [`Self::outcome`] / [`Self::will_retry`]. A hook may leave it, add effects
/// (committed with it, also with a retry), or replace it with a final outcome, which cancels the retry.
pub struct HandlerErrorContext<'a> {
    pub(crate) info: &'a TurnInfo,
    pub(crate) sender: RawSender<'a>,
    pub(crate) error: &'a HandlerFailure,
    pub(crate) outcome: TurnOutcome,
    pub(crate) retry: Option<TurnOutcome>,
    pub(crate) retry_delay: Option<Duration>,
    pub(crate) failure_reason: String,
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

    /// The outcome to commit: by default the runtime's decision (rescheduled when the message is retried, else
    /// `failed` with [`Self::failure_reason`]).
    pub fn outcome(&self) -> &TurnOutcome {
        &self.outcome
    }

    /// Replaces the outcome with a final one ([`TurnOutcome::Done`] or [`TurnOutcome::Failed`]), which also cancels a
    /// retry; or restores the runtime's own retry decision. Any other reschedule is refused: hooks cannot schedule
    /// retries.
    pub fn set_outcome(&mut self, outcome: TurnOutcome) -> Result<(), String> {
        if !outcome.is_final() && Some(&outcome) != self.retry.as_ref() {
            return Err(format!(
                "An on_handler_error hook cannot schedule a retry ({outcome:?}); retry is set by the message's sender"
            ));
        }
        self.outcome = outcome;
        Ok(())
    }

    /// True while the outcome is the runtime's retry.
    pub fn will_retry(&self) -> bool {
        !self.outcome.is_final()
    }

    /// The backoff before the next attempt that the runtime decided; `None` when it decided not to retry.
    pub fn retry_delay(&self) -> Option<Duration> {
        self.retry_delay
    }

    /// The reason the failure is recorded with (also as `lastError` of a retry).
    pub fn failure_reason(&self) -> &str {
        &self.failure_reason
    }

    /// Buffers a send of `message_type` with `payload` to `to`, checked against the TypeRegistry now and applied at
    /// commit.
    pub async fn send(
        &mut self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: HookSend,
    ) -> Result<MessageId, TurnError> {
        self.sender.send(self.info, to, message_type, payload, options).await
    }
}

/// Context of [`Plugin::before_commit`].
pub struct BeforeCommitContext<'a> {
    pub(crate) info: &'a TurnInfo,
    pub(crate) sender: RawSender<'a>,
}

impl BeforeCommitContext<'_> {
    /// The turn.
    pub fn info(&self) -> &TurnInfo {
        self.info
    }

    /// Buffers a send, as [`HandlerErrorContext::send`].
    pub async fn send(
        &mut self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: HookSend,
    ) -> Result<MessageId, TurnError> {
        self.sender.send(self.info, to, message_type, payload, options).await
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
        assert_eq!(
            a.with_mut::<u32, _>(|v| {
                *v += 1;
                *v
            }),
            Some(6)
        );
        assert_eq!(a.remove::<TraceId>(), Some(TraceId("t1".into())));
        assert!(!b.contains::<TraceId>());
    }

    #[test]
    fn reserved_headers() {
        assert!(reserved_headers::is_reserved("cicada-trace"));
        assert!(reserved_headers::is_reserved("Cicada-Trace"));
        assert!(!reserved_headers::is_reserved("trace"));
        assert!(!reserved_headers::is_reserved("cic"));
    }
}
