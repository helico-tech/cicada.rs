//! What a handler sees and does during one turn.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::actor::Actor;
use crate::address::{ActorAddress, ActorRef};
use crate::message::{self, MessageFamily};
use crate::plugin::{Attributes, TurnInfo};
use crate::retention::Retention;
use crate::retry::RetryPolicy;
use crate::runtime::effects::EffectBuffer;
use crate::runtime::limits;
use crate::runtime::resolver::TypeResolver;
use crate::store::{ActorCreate, Control, ControlEffect, Delivery, Message, MessageId, OutgoingMessage, StoreError};
use crate::time::{self, Timestamp};
use crate::watch::{IntoWatchTarget, WatchChange};

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
            HandlerFailure::Timeout { max_turn, .. } => {
                write!(f, "timeout: the turn exceeded its maximum duration of {}", time::format_duration(*max_turn))
            }
            other => f.write_str(&other.reason()),
        }
    }
}

/// A send, create, cancel, watch or control effect of a handler (or hook) that was refused when called.
#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    /// A field is out of bounds, contains a NUL character, the value cannot be encoded, or an initial state is `null`;
    /// or a watch whose target is no target, or of an actor whose type may not watch; or a delete of the turn's own
    /// actor, or of an actor the turn creates already.
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
    /// Headers passed through unchanged. A plugin may add headers to a turn's sends (trace context); a header set
    /// here wins over one of the same name from a plugin.
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
}

/// Checks a delivery's bounds when a send is called.
pub(crate) fn check_delivery(delivery: &Delivery) -> Result<(), TurnError> {
    match *delivery {
        Delivery::Now => Ok(()),
        Delivery::At(t) => Delivery::at(t).map(|_| ()).map_err(|e| TurnError::Invalid(e.0)),
        Delivery::After(d) => Delivery::after(d).map(|_| ()).map_err(|e| TurnError::Invalid(e.0)),
    }
}

/// What a handler sees during one turn. Everything it does inside the system is buffered and applied by the turn's
/// single atomic commit; nothing is visible before that.
///
/// Every send checks its envelope when called and fails with [`TurnError::Invalid`] (which the handler can handle)
/// for fields beyond the send limits and for NUL characters.
pub struct Turn<A: Actor> {
    pub(crate) info: Arc<TurnInfo>,
    self_ref: A::Ref,
    state: A::State,
    state_assigned: bool,
    pub(crate) effects: EffectBuffer,
    resolver: Arc<TypeResolver>,
    /// Why this actor's type may not watch; `None` when it may.
    watch_problem: Option<String>,
}

impl<A: Actor> Turn<A> {
    pub(crate) fn new(
        info: Arc<TurnInfo>,
        self_ref: A::Ref,
        state: A::State,
        resolver: Arc<TypeResolver>,
        watch_problem: Option<String>,
    ) -> Self {
        Self { info, self_ref, state, state_assigned: false, effects: EffectBuffer::default(), resolver, watch_problem }
    }

    pub(crate) fn into_parts(self) -> (A::State, bool, EffectBuffer) {
        (self.state, self.state_assigned, self.effects)
    }

    /// Own typed ref, read from this instance's name.
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
    /// increased.
    pub fn state_mut(&mut self) -> &mut A::State {
        self.state_assigned = true;
        &mut self.state
    }

    /// Replaces the state; written at commit.
    pub fn set_state(&mut self, state: A::State) {
        self.state_assigned = true;
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
        let address = to.address().map_err(|e| TurnError::Invalid(e.to_string()))?;
        self.buffer(address, &msg.into(), options)
    }

    /// Buffers `msg` for this instance itself, due now.
    pub fn send_self(&mut self, msg: impl Into<<A::Ref as ActorRef>::Message>) -> Result<MessageId, TurnError> {
        self.send_self_with(msg, SendOptions::default())
    }

    /// Buffers `msg` for this instance itself with `options`.
    pub fn send_self_with(
        &mut self,
        msg: impl Into<<A::Ref as ActorRef>::Message>,
        options: SendOptions,
    ) -> Result<MessageId, TurnError> {
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
        let encoded = message::encode(msg).map_err(|e| TurnError::Invalid(e.to_string()))?;
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
        let options = self.with_turn_headers(options);
        let id = check_send(message_type, &payload, &options)?;
        self.resolver.check_send(to, message_type).await?;
        self.push_send(to.clone(), message_type.to_owned(), payload, id.clone(), options);
        Ok(id)
    }

    /// Buffers an idempotent create of `r` with `initial_state`, as an independent actor: it has no parent, and
    /// removing this instance leaves it. Creates are applied before the turn's sends, so create-then-send in one turn
    /// works. An existing instance is left untouched. A state that encodes to `null` is refused with
    /// [`TurnError::Invalid`]; a type that keeps no data is created with [`crate::NoState`]. Fails with
    /// [`TurnError::Invalid`] too when this turn creates `r` with [`Self::create_child`] already.
    pub fn create<R: ActorRef, S: Serialize + ?Sized>(&mut self, r: &R, initial_state: &S) -> Result<(), TurnError> {
        self.buffer_create(r, initial_state, false)
    }

    /// Buffers an idempotent create of `r` with `initial_state`, as a child of this instance: its parent is this
    /// instance, for life, and deleting or stopping this instance removes it too, with its own descendants (ADR 0019).
    /// It takes the arguments of [`Self::create`] and is checked the same way, and it is applied before the turn's
    /// sends as well.
    ///
    /// A child of this instance that exists already is left untouched. An actor that exists with another parent, or
    /// with none (created with [`Self::create`] or from outside a turn), is never taken over: the commit is rejected
    /// and the turn's message ends `failed`, as for a send to an absent actor; so is a create of this instance itself.
    /// Fails with [`TurnError::Invalid`] when this turn creates `r` with [`Self::create`] already.
    pub fn create_child<R: ActorRef, S: Serialize + ?Sized>(
        &mut self,
        r: &R,
        initial_state: &S,
    ) -> Result<(), TurnError> {
        self.buffer_create(r, initial_state, true)
    }

    fn buffer_create<R: ActorRef, S: Serialize + ?Sized>(
        &mut self,
        r: &R,
        initial_state: &S,
        child: bool,
    ) -> Result<(), TurnError> {
        let state = serde_json::to_value(initial_state)
            .map_err(|e| TurnError::Invalid(format!("The initial state cannot be encoded: {e}")))?;
        message::check_not_null("initial state", &state).map_err(TurnError::Invalid)?;
        limits::check_json("initial state", Some(&state)).map_err(TurnError::Invalid)?;
        let address = r.address().map_err(|e| TurnError::Invalid(e.to_string()))?;
        if self.effects.creates.iter().any(|c| c.address == address && c.child != child) {
            let (first, second) = if child { ("create", "create_child") } else { ("create_child", "create") };
            return Err(TurnError::Invalid(format!(
                "{address} is created with {first} in this turn already; {second} of it would disagree"
            )));
        }
        self.effects.creates.push(ActorCreate { address, state, child });
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

    /// Watches `target` from this instance at commit (ADR 0023): a typed ref, an address, a [`WatchTarget`] or its text
    /// (`billing.invoices/acme`, a type prefix such as `billing`, or `*` for every type). From then on the store puts a
    /// [`Failed`](crate::Failed) into this instance's mailbox when a message to a matching actor ends `failed`, and a
    /// [`Stopped`](crate::Stopped) when the watched actor itself goes away; a watch on a type prefix or on every type
    /// tells of failures only. The target need not exist, and the watch survives its delete and re-create.
    ///
    /// Buffered and applied by the commit before the turn's creates and sends, in the order of the calls; a turn that
    /// fails sets none. Setting a watch that exists changes nothing. Fails with [`TurnError::Invalid`] for a text that
    /// is no target, and when this actor's message family does not declare `Failed(cicada::Failed)` and
    /// `Stopped(cicada::Stopped)`; the turn can go on.
    ///
    /// [`WatchTarget`]: crate::WatchTarget
    pub fn watch(&mut self, target: impl IntoWatchTarget) -> Result<(), TurnError> {
        let target = target.into_watch_target().map_err(|e| TurnError::Invalid(e.to_string()))?;
        if let Some(problem) = &self.watch_problem {
            return Err(TurnError::Invalid(problem.clone()));
        }
        self.effects.watches.push(WatchChange { target, set: true });
        Ok(())
    }

    /// Removes this instance's watch of `target` at commit, in the order of the calls with [`Self::watch`], so the last
    /// call for a target stands. A watch it does not hold is ignored. Fails with [`TurnError::Invalid`] for a text that
    /// is no target.
    pub fn unwatch(&mut self, target: impl IntoWatchTarget) -> Result<(), TurnError> {
        let target = target.into_watch_target().map_err(|e| TurnError::Invalid(e.to_string()))?;
        self.effects.watches.push(WatchChange { target, set: false });
        Ok(())
    }

    /// Deletes this instance at commit, with every descendant: the actors it created with [`Self::create_child`],
    /// theirs, and so on, the children of this turn included. Their remaining mailboxes are purged, this turn's sends
    /// to itself and to them included, and a turn running on a descendant commits nothing.
    pub fn stop(&mut self) {
        self.effects.stop = true;
    }

    /// Pauses `actor` at commit, as [`ActorSystem::pause_mailbox`](crate::ActorSystem::pause_mailbox) does from outside
    /// (ADR 0024): its messages queue up, no new turn of it is claimed until it is resumed, and a turn running on it
    /// finishes. Any actor, this one included; one that is not there at commit is left alone, and the commit does not
    /// fail for it.
    ///
    /// Control effects are buffered and applied by the commit together with the turn's other effects, or not at all,
    /// by kind and not in the order of the calls: watches, deletes, creates, purges, pauses and resumes, then sends. So a
    /// turn can create an actor, pause it and send it a message that waits. After a pause and a resume of one actor the
    /// last call stands. Fails with [`TurnError::Invalid`] for a ref that has no address.
    pub fn pause<R: ActorRef>(&mut self, actor: &R) -> Result<(), TurnError> {
        self.control(ControlEffect::Pause, &address_of(actor)?)
    }

    /// [`Self::pause`] of the actor at `actor`, such as the actor of a [`Stopped`](crate::Stopped).
    pub fn pause_at(&mut self, actor: &ActorAddress) -> Result<(), TurnError> {
        self.control(ControlEffect::Pause, actor)
    }

    /// Resumes `actor` at commit, as [`ActorSystem::resume_mailbox`](crate::ActorSystem::resume_mailbox) does: its turns
    /// are claimed again. Applied as [`Self::pause`] is; after a pause and a resume of one actor the last call stands.
    pub fn resume<R: ActorRef>(&mut self, actor: &R) -> Result<(), TurnError> {
        self.control(ControlEffect::Resume, &address_of(actor)?)
    }

    /// [`Self::resume`] of the actor at `actor`.
    pub fn resume_at(&mut self, actor: &ActorAddress) -> Result<(), TurnError> {
        self.control(ControlEffect::Resume, actor)
    }

    /// Purges the mailbox of `actor` at commit, as [`ActorSystem::purge_mailbox`](crate::ActorSystem::purge_mailbox)
    /// does: every pending message ends `purged`, except one that is being handled, and the actor stays with its state.
    /// Purges come before sends, so a turn can empty a mailbox and put a first message into it. Of this actor, the
    /// message of this turn is the one being handled. Applied as [`Self::pause`] is.
    pub fn purge<R: ActorRef>(&mut self, actor: &R) -> Result<(), TurnError> {
        self.control(ControlEffect::Purge, &address_of(actor)?)
    }

    /// [`Self::purge`] of the actor at `actor`.
    pub fn purge_at(&mut self, actor: &ActorAddress) -> Result<(), TurnError> {
        self.control(ControlEffect::Purge, actor)
    }

    /// Deletes `actor` at commit, as [`ActorSystem::delete_actor`](crate::ActorSystem::delete_actor) does (ADR 0024):
    /// the actor, its mailbox and its descendants go, every pending message ends `purged`, and a turn running on one of
    /// them commits nothing and its message ends `purged`, while what its handler did outside the store stays done.
    /// The watchers of each removed actor get a [`Stopped`](crate::Stopped) with `by` this actor: the cause is
    /// `deleted` for an actor this turn deletes, `parent` for a descendant. Any actor, also one this one did not
    /// create; one that is not there at commit is left alone, and the commit does not fail for it.
    ///
    /// Deletes come before creates, so a delete and then a create of one actor restart it: the new actor is another
    /// life, with the new state, an empty mailbox and no watches of its own, and a handler of the old life may still
    /// run for a moment, committing nothing. A parent restarts its child with [`Self::create_child`]; restarted with
    /// [`Self::create`] by another actor, the child loses its descendants and has no parent. A send to an actor that
    /// is deleted and not created again rejects the commit, as for any absent actor.
    ///
    /// Fails with [`TurnError::Invalid`], and buffers nothing, for a ref that has no address, for this actor (a turn
    /// stops itself with [`Self::stop`]) and after a create of the same actor in this turn (a restart is a delete,
    /// then a create). A delete of an ancestor of this actor rejects the commit, and the turn's message ends `failed`.
    pub fn delete<R: ActorRef>(&mut self, actor: &R) -> Result<(), TurnError> {
        self.control(ControlEffect::Delete, &address_of(actor)?)
    }

    /// [`Self::delete`] of the actor at `actor`, such as the actor of a [`Stopped`](crate::Stopped).
    pub fn delete_at(&mut self, actor: &ActorAddress) -> Result<(), TurnError> {
        self.control(ControlEffect::Delete, actor)
    }

    /// Buffers a control effect: each effect once per actor, and of a pause and a resume of one actor the last.
    fn control(&mut self, effect: ControlEffect, actor: &ActorAddress) -> Result<(), TurnError> {
        if effect == ControlEffect::Delete {
            if actor == &self.info.self_address {
                return Err(TurnError::Invalid(format!(
                    "A turn cannot delete its own actor {actor}; it stops itself with turn.stop()"
                )));
            }
            if self.effects.creates.iter().any(|c| c.address == *actor) {
                return Err(TurnError::Invalid(format!(
                    "{actor} is created in this turn already, and deletes are applied before creates; a restart is a \
                     delete, then a create"
                )));
            }
        }
        let controls = &mut self.effects.controls;
        match effect {
            ControlEffect::Pause | ControlEffect::Resume => controls
                .retain(|c| c.actor != *actor || !matches!(c.effect, ControlEffect::Pause | ControlEffect::Resume)),
            ControlEffect::Purge | ControlEffect::Delete => {
                if controls.iter().any(|c| c.effect == effect && c.actor == *actor) {
                    return Ok(());
                }
            }
        }
        controls.push(Control { effect, actor: actor.clone() });
        Ok(())
    }

    /// A permanent failure with `reason`, for `return Err(turn.fail("declined"))`: the message ends `failed` now,
    /// without retry whatever its retry policy.
    pub fn fail(&self, reason: impl fmt::Display + fmt::Debug + Send + Sync + 'static) -> HandlerError {
        HandlerError::permanent(reason)
    }

    fn buffer<M: MessageFamily>(
        &mut self,
        to: ActorAddress,
        msg: &M,
        options: SendOptions,
    ) -> Result<MessageId, TurnError> {
        let encoded = message::encode(msg).map_err(|e| TurnError::Invalid(e.to_string()))?;
        let options = self.with_turn_headers(options);
        let id = check_send(&encoded.message_type, &encoded.payload, &options)?;
        self.push_send(to, encoded.message_type, encoded.payload, id.clone(), options);
        Ok(id)
    }

    /// Adds the headers that plugins set for this turn's sends; the send's own headers win.
    fn with_turn_headers(&self, mut options: SendOptions) -> SendOptions {
        let turn_headers = self.info.send_headers.lock().unwrap_or_else(|e| e.into_inner());
        for (name, value) in turn_headers.iter() {
            options.headers.entry(name.clone()).or_insert_with(|| value.clone());
        }
        options
    }

    fn push_send(
        &mut self,
        to: ActorAddress,
        message_type: String,
        payload: Value,
        id: MessageId,
        options: SendOptions,
    ) {
        self.effects.sends.push(OutgoingMessage {
            to,
            message_type,
            payload,
            id,
            from: Some(self.info.self_address.clone()),
            delivery: options.delivery,
            correlation_id: options.correlation_id,
            headers: options.headers,
            retry: options.retry,
            caused_by: Some(self.info.message.id.clone()),
            retention: options.retention,
        });
    }
}

/// The address of `actor`; [`TurnError::Invalid`] for a ref that has none.
fn address_of<R: ActorRef>(actor: &R) -> Result<ActorAddress, TurnError> {
    actor.address().map_err(|e| TurnError::Invalid(e.to_string()))
}

/// Checks the envelope of a send when it is called; returns the message's id (the given one, or a fresh one).
fn check_send(message_type: &str, payload: &Value, options: &SendOptions) -> Result<MessageId, TurnError> {
    check_delivery(&options.delivery)?;
    let id = options.id.clone().unwrap_or_else(MessageId::random);
    limits::check_envelope(&id, message_type, payload, options.correlation_id.as_deref(), &options.headers)
        .map_err(TurnError::Invalid)?;
    Ok(id)
}

impl<A: Actor> fmt::Debug for Turn<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Turn").field("self", &self.info.self_address).field("message", &self.info.message.id).finish()
    }
}
