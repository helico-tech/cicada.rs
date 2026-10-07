//! What a handler sees and does during one turn.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::actor::Actor;
use crate::address::{ActorAddress, ActorRef, RefBinding, to_params};
use crate::message::{self, ActorState, MessageFamily};
use crate::plugin::{Attributes, HookSend, TurnInfo, reserved_headers};
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::runtime::effects::{EffectBuffer, RawSender};
use crate::runtime::limits;
use crate::runtime::resolver::TypeResolver;
use crate::store::{ActorCreate, Delivery, Message, MessageId, OutgoingMessage, StoreError};
use crate::time::{self, Timestamp};

/// The result of a handler.
pub type HandlerResult = Result<(), HandlerError>;

/// A handler failure: an ordinary failure (retried under the message's retry policy) or a permanent one (the
/// receiver's veto: the message ends `failed` at once, whatever its retry policy).
///
/// Any error converts into an ordinary failure with `?`. Build a permanent one with [`HandlerError::permanent`]
/// (or [`Turn::fail`]): `return Err(turn.fail("declined"))`.
pub struct HandlerError {
    permanent: bool,
    error: anyhow::Error,
}

impl HandlerError {
    /// An ordinary failure with a message.
    pub fn msg(message: impl fmt::Display + fmt::Debug + Send + Sync + 'static) -> Self {
        Self { permanent: false, error: anyhow::Error::msg(message) }
    }

    /// An ordinary failure from any error, including an [`anyhow::Error`].
    pub fn new(error: impl Into<anyhow::Error>) -> Self {
        Self { permanent: false, error: error.into() }
    }

    /// A permanent failure: the message ends `failed` with `reason` now, without retry.
    pub fn permanent(reason: impl fmt::Display + fmt::Debug + Send + Sync + 'static) -> Self {
        Self { permanent: true, error: anyhow::Error::msg(reason) }
    }

    /// A permanent failure caused by `error`; its message is the reason.
    pub fn permanent_from(error: impl Into<anyhow::Error>) -> Self {
        Self { permanent: true, error: error.into() }
    }

    /// True for a permanent failure.
    pub fn is_permanent(&self) -> bool {
        self.permanent
    }

    /// The underlying error.
    pub fn error(&self) -> &anyhow::Error {
        &self.error
    }

    /// The reason the failure is recorded with: the message of a permanent failure, the error chain of an ordinary
    /// one.
    pub fn reason(&self) -> String {
        if self.permanent { self.error.to_string() } else { format!("{:#}", self.error) }
    }
}

impl<E: std::error::Error + Send + Sync + 'static> From<E> for HandlerError {
    fn from(error: E) -> Self {
        Self { permanent: false, error: error.into() }
    }
}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason())
    }
}

impl fmt::Debug for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandlerError").field("permanent", &self.permanent).field("error", &self.error).finish()
    }
}

/// How a handler run failed, as `on_handler_error` hooks see it.
#[derive(Debug)]
pub enum HandlerFailure {
    /// The handler returned an error.
    Error(HandlerError),
    /// The handler panicked.
    Panic(String),
    /// The handler ran longer than its type's maximum turn duration and was cancelled (or only ended after it, in
    /// code that does not yield; then `cause` is its own failure, if it failed).
    Timeout {
        /// The maximum turn duration that was exceeded.
        max_turn: Duration,
        /// The handler's own failure when it failed only after overrunning.
        cause: Option<Box<HandlerFailure>>,
    },
}

impl HandlerFailure {
    /// True for a permanent failure (the receiver's veto).
    pub fn is_permanent(&self) -> bool {
        matches!(self, HandlerFailure::Error(e) if e.is_permanent())
    }

    /// True for a turn timeout.
    pub fn is_timeout(&self) -> bool {
        matches!(self, HandlerFailure::Timeout { .. })
    }

    /// The reason the failure is recorded with: the handler's error, `panic: …`, or `timeout`.
    pub fn reason(&self) -> String {
        match self {
            HandlerFailure::Error(e) => e.reason(),
            HandlerFailure::Panic(m) => format!("panic: {m}"),
            HandlerFailure::Timeout { .. } => "timeout".to_owned(),
        }
    }
}

impl fmt::Display for HandlerFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandlerFailure::Timeout { max_turn, .. } => write!(
                f,
                "timeout: the turn exceeded its maximum duration of {}",
                time::format_duration(*max_turn)
            ),
            other => f.write_str(&other.reason()),
        }
    }
}

/// A send, create or cancel of a handler (or hook) that was refused when called.
#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    /// A field is out of bounds, contains a NUL character, uses a reserved header, or the value cannot be encoded.
    #[error("{0}")]
    Invalid(String),
    /// A send to a raw address was refused: no registered type matches it, or that type does not accept the message
    /// type.
    #[error("{reason}")]
    SendRejected {
        /// The target.
        to: ActorAddress,
        /// The message type name.
        message_type: String,
        /// Why.
        reason: String,
    },
    /// The TypeRegistry could not be read to check a raw send.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Options of a send: delivery time, correlation id, headers, retry and retention policy, and the id.
///
/// ```
/// use std::time::Duration;
/// use cicada::turn::SendOptions;
///
/// let options = SendOptions::new().delay(Duration::from_secs(3600)).correlation_id("run-7");
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendOptions {
    /// When the message becomes due; now by default.
    pub delivery: Delivery,
    /// Opaque string passed through unchanged.
    pub correlation_id: Option<String>,
    /// Plugin headers passed through unchanged; names starting with `cicada-` are reserved.
    pub headers: BTreeMap<String, String>,
    /// The new message's retry policy; `None`: a failed handler run of it ends it `failed`.
    pub retry: Option<RetryPolicy>,
    /// How long the new message's record is kept once it finished; `None` for the store default.
    pub retention: Option<Retention>,
    /// The message id (the dedup key); a fresh one when `None`.
    pub id: Option<MessageId>,
}

impl SendOptions {
    /// Default options: due now, no policies.
    pub fn new() -> Self {
        Self::default()
    }

    /// Due `delay` after the commit (store time at commit).
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delivery = Delivery::After(delay);
        self
    }

    /// Due at `instant` (clamped to store now). Absolute: count from [`Turn::current_time`] or use a delay to count
    /// from the end of the work.
    pub fn at(mut self, instant: Timestamp) -> Self {
        self.delivery = Delivery::At(instant);
        self
    }

    /// Sets the correlation id.
    pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Adds a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Sets the retry policy.
    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry = Some(policy);
        self
    }

    /// Sets the retention policy.
    pub fn retention(mut self, retention: Retention) -> Self {
        self.retention = Some(retention);
        self
    }

    /// Sets the message id.
    pub fn id(mut self, id: MessageId) -> Self {
        self.id = Some(id);
        self
    }

    pub(crate) fn into_hook_send(self) -> HookSend {
        HookSend {
            delivery: self.delivery,
            correlation_id: self.correlation_id,
            headers: self.headers,
            id: self.id,
            retry: self.retry,
            retention: self.retention,
        }
    }
}

/// Checks a delivery's bounds when a send is called.
pub(crate) fn check_delivery(delivery: &Delivery) -> Result<(), TurnError> {
    match *delivery {
        Delivery::Now => Ok(()),
        Delivery::At(t) => Delivery::at(t).map(|_| ()).map_err(|e| TurnError::Invalid(e.0)),
        Delivery::After(d) => Delivery::after(d).map(|_| ()).map_err(|e| TurnError::Invalid(e.0)),
    }
}

pub(crate) fn refuse_reserved(headers: &BTreeMap<String, String>) -> Result<(), TurnError> {
    if let Some(name) = headers.keys().find(|n| reserved_headers::is_reserved(n)) {
        return Err(TurnError::Invalid(format!(
            "Header '{name}' uses the reserved prefix '{}'; only plugins may set such headers",
            reserved_headers::PREFIX
        )));
    }
    Ok(())
}

/// What a handler sees during one turn. Everything it does inside the system is buffered and applied by the turn's
/// single atomic commit; nothing is visible before that.
///
/// Every send checks its envelope when called and fails with [`TurnError::Invalid`] (which the handler can handle)
/// for a header with the reserved prefix `cicada-`, for fields beyond the send limits, and for NUL characters.
pub struct Turn<A: Actor> {
    pub(crate) info: Arc<TurnInfo>,
    self_ref: A::Ref,
    state: A::State,
    state_assigned: bool,
    pub(crate) effects: EffectBuffer,
    resolver: Arc<TypeResolver>,
}

impl<A: Actor> Turn<A> {
    pub(crate) fn new(info: Arc<TurnInfo>, self_ref: A::Ref, state: A::State, resolver: Arc<TypeResolver>) -> Self {
        Self { info, self_ref, state, state_assigned: false, effects: EffectBuffer::default(), resolver }
    }

    pub(crate) fn into_parts(self) -> (A::State, bool, EffectBuffer) {
        (self.state, self.state_assigned, self.effects)
    }

    /// Own typed ref. Its fields are the template parameters.
    pub fn self_ref(&self) -> &A::Ref {
        &self.self_ref
    }

    /// Own address.
    pub fn self_address(&self) -> &ActorAddress {
        &self.info.self_address
    }

    /// Current state.
    pub fn state(&self) -> &A::State {
        &self.state
    }

    /// Mutable access to the state. Taking it marks the state as written: it is stored at commit and the version is
    /// increased (for a stateful type).
    pub fn state_mut(&mut self) -> &mut A::State {
        self.state_assigned = !A::State::STATELESS;
        &mut self.state
    }

    /// Replaces the state; written at commit.
    pub fn set_state(&mut self, state: A::State) {
        self.state_assigned = !A::State::STATELESS;
        self.state = state;
    }

    /// The envelope being handled: `id`, `from`, `message_type`, `attempt`, `failures`, `last_error`, `retry`,
    /// `deliver_at`, `correlation_id`, `headers`. A retried message is the same message, so `id` is a stable
    /// idempotency key for external calls.
    pub fn message(&self) -> &Message {
        &self.info.message
    }

    /// Store time at claim. It does not move while the handler runs.
    pub fn now(&self) -> Timestamp {
        self.info.now
    }

    /// `now` plus the time that passed on this node since the claim (its monotonic clock, so the virtual time in
    /// tests). Use it for absolute times that should count from the end of the work.
    pub fn current_time(&self) -> Timestamp {
        self.info.current_time()
    }

    /// The typed key-value bag shared with plugins, valid for this turn only.
    pub fn attributes(&self) -> &Attributes {
        &self.info.attributes
    }

    /// Buffers `msg` for `to`, due now. The compiler checks that `to` accepts `msg`; the recipient must exist at
    /// commit, or the whole turn fails. Returns the new message's id.
    pub fn send<R: ActorRef>(&mut self, to: &R, msg: impl Into<R::Message>) -> Result<MessageId, TurnError> {
        self.send_with(to, msg, SendOptions::default())
    }

    /// Buffers `msg` for `to` with `options` (delay or time, correlation id, headers, retry, retention).
    pub fn send_with<R: ActorRef>(
        &mut self,
        to: &R,
        msg: impl Into<R::Message>,
        options: SendOptions,
    ) -> Result<MessageId, TurnError> {
        let address = self.typed_address(to)?;
        self.buffer(address, &msg.into(), options)
    }

    /// Buffers `msg` for this instance itself, due now.
    pub fn send_self(&mut self, msg: impl Into<A::Message>) -> Result<MessageId, TurnError> {
        self.send_self_with(msg, SendOptions::default())
    }

    /// Buffers `msg` for this instance itself with `options`.
    pub fn send_self_with(&mut self, msg: impl Into<A::Message>, options: SendOptions) -> Result<MessageId, TurnError> {
        let address = self.info.self_address.clone();
        self.buffer(address, &msg.into(), options)
    }

    /// Buffers `msg` (of any family) for the raw address `to`, such as the sender of the current message. The type name
    /// is checked against the TypeRegistry now, so the handler can handle a [`TurnError::SendRejected`]; the recipient
    /// must exist at commit.
    pub async fn send_to<M: MessageFamily>(
        &mut self,
        to: &ActorAddress,
        msg: &M,
        options: SendOptions,
    ) -> Result<MessageId, TurnError> {
        let encoded = message::encode(msg).map_err(TurnError::Invalid)?;
        self.send_raw(to, &encoded.message_type, encoded.payload, options).await
    }

    /// Buffers a message given as type name and JSON payload for `to`. The type name is checked against the
    /// TypeRegistry now; the payload is decoded by the recipient at delivery.
    pub async fn send_raw(
        &mut self,
        to: &ActorAddress,
        message_type: &str,
        payload: Value,
        options: SendOptions,
    ) -> Result<MessageId, TurnError> {
        refuse_reserved(&options.headers)?;
        let mut sender = RawSender { resolver: &self.resolver, effects: &mut self.effects, allow_reserved: false };
        sender.send(&self.info, to, message_type, payload, options.into_hook_send()).await
    }

    /// Buffers an idempotent create of `r` with `initial_state`. Creates are applied before the turn's sends, so
    /// create-then-send in one turn works. An existing instance is left untouched.
    pub fn create<R: ActorRef, S: Serialize + ?Sized>(&mut self, r: &R, initial_state: &S) -> Result<(), TurnError> {
        let state = serde_json::to_value(initial_state)
            .map_err(|e| TurnError::Invalid(format!("The initial state cannot be encoded: {e}")))?;
        limits::check_json("initial state", Some(&state)).map_err(TurnError::Invalid)?;
        self.push_create(r, Some(state))
    }

    /// Buffers an idempotent create of `r` without state (for a stateless type).
    pub fn create_stateless<R: ActorRef>(&mut self, r: &R) -> Result<(), TurnError> {
        self.push_create(r, None)
    }

    fn push_create<R: ActorRef>(&mut self, r: &R, state: Option<Value>) -> Result<(), TurnError> {
        let binding = RefBinding::<R>::new().map_err(|e| TurnError::Invalid(e.to_string()))?;
        let address = binding.to_address(r).map_err(|e| TurnError::Invalid(e.to_string()))?;
        self.effects.creates.push(ActorCreate { address, template: binding.template().clone(), state });
        Ok(())
    }

    /// Cancels the pending message `id` of this instance at commit (outcome `cancelled`), atomically with the turn's
    /// other effects. An id that is not a pending message of this instance by then is ignored. A message sent earlier
    /// in this turn is not sent at all. Fails for the turn's own message.
    pub fn cancel(&mut self, id: &MessageId) -> Result<(), TurnError> {
        if id == &self.info.message.id {
            return Err(TurnError::Invalid(format!(
                "A turn cannot cancel its own message {id}; end it with the handler's outcome"
            )));
        }
        let before = self.effects.sends.len();
        self.effects.sends.retain(|s| &s.id != id);
        if self.effects.sends.len() == before && !self.effects.cancels.contains(id) {
            self.effects.cancels.push(id.clone());
        }
        Ok(())
    }

    /// Deletes this instance at commit; its remaining mailbox (including this turn's self-sends) is purged.
    pub fn stop(&mut self) {
        self.effects.stop = true;
    }

    /// A permanent failure with `reason`, for `return Err(turn.fail("declined"))`: the message ends `failed` now,
    /// without retry whatever its retry policy.
    pub fn fail(&self, reason: impl fmt::Display + fmt::Debug + Send + Sync + 'static) -> HandlerError {
        HandlerError::permanent(reason)
    }

    fn typed_address<R: ActorRef>(&mut self, to: &R) -> Result<ActorAddress, TurnError> {
        let invalid = |e: &dyn fmt::Display| TurnError::Invalid(e.to_string());
        // `self` always means this instance's own address, even when it was claimed under a non-canonical form.
        if R::TEMPLATE == <A::Ref as ActorRef>::TEMPLATE
            && let (Ok(a), Ok(b)) = (to_params(to), to_params(&self.self_ref))
            && a == b
        {
            return Ok(self.info.self_address.clone());
        }
        let binding = RefBinding::<R>::new().map_err(|e| invalid(&e))?;
        let address = binding.to_address(to).map_err(|e| invalid(&e))?;
        if !self.effects.typed_targets.iter().any(|(a, _)| a == &address) {
            self.effects.typed_targets.push((address.clone(), binding.template().clone()));
        }
        Ok(address)
    }

    fn buffer<M: MessageFamily>(
        &mut self,
        to: ActorAddress,
        msg: &M,
        options: SendOptions,
    ) -> Result<MessageId, TurnError> {
        refuse_reserved(&options.headers)?;
        check_delivery(&options.delivery)?;
        let encoded = message::encode(msg).map_err(TurnError::Invalid)?;
        let id = options.id.unwrap_or_else(MessageId::random);
        limits::check_envelope(&id, &encoded.message_type, &encoded.payload, options.correlation_id.as_deref(), &options.headers)
            .map_err(TurnError::Invalid)?;
        self.effects.sends.push(OutgoingMessage {
            to,
            message_type: encoded.message_type,
            payload: encoded.payload,
            id: id.clone(),
            from: Some(self.info.self_address.clone()),
            delivery: options.delivery,
            correlation_id: options.correlation_id,
            headers: options.headers,
            retry: options.retry,
            caused_by: Some(self.info.message.id.clone()),
            retention: options.retention,
        });
        Ok(id)
    }
}

impl<A: Actor> fmt::Debug for Turn<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Turn").field("self", &self.info.self_address).field("message", &self.info.message.id).finish()
    }
}
