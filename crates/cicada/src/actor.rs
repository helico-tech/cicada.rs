//! Actor types: the [`Actor`] trait a user implements, and [`ActorType`], which a node hosts.

use std::any::type_name;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::Value;

use crate::address::{ActorAddress, ActorRef, RefBinding, UriTemplate};
use crate::contract::Contract;
use crate::message::{self, ActorState, FamilyInfo, MessageFamily};
use crate::plugin::TurnInfo;
use crate::runtime::effects::EffectBuffer;
use crate::runtime::resolver::TypeResolver;
use crate::store::Claim;
use crate::turn::{HandlerFailure, HandlerResult, Turn};

/// The mandatory policy for messages that cannot be decoded at delivery (an unknown type name or an invalid payload).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unhandled {
    /// End the message `failed` with the decoding error.
    Fail,
    /// End the message `done`, without effects.
    Drop,
}

/// An actor type: a typed ref bound to a URI template, an optional state, a message family, one handler for the whole
/// family, and the mandatory unhandled policy.
///
/// ```
/// use cicada::prelude::*;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// struct Counter {
///     id: String,
/// }
///
/// impl ActorRef for Counter {
///     const TEMPLATE: &'static str = "actor://counter/{id}";
///     type Message = CounterMsg;
/// }
///
/// #[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// #[serde(tag = "type")]
/// enum CounterMsg {
///     Add { amount: i64 },
///     Reset,
/// }
///
/// #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
/// struct CounterState {
///     total: i64,
/// }
///
/// struct CounterActor;
///
/// impl Actor for CounterActor {
///     type Ref = Counter;
///     type State = CounterState;
///     type Message = CounterMsg;
///     const UNHANDLED: Unhandled = Unhandled::Fail;
///
///     async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
///         match message {
///             CounterMsg::Add { amount } => turn.state_mut().total += amount,
///             CounterMsg::Reset => turn.set_state(CounterState::default()),
///         }
///         Ok(())
///     }
/// }
/// ```
///
/// The `match` over the message family replaces a per-message handler registry: the compiler checks that every
/// message is handled. The value implementing the trait carries the actor's dependencies (clients, configuration),
/// shared by all its turns.
pub trait Actor: Send + Sync + Sized + 'static {
    /// The typed ref; its template is the type's identity.
    type Ref: ActorRef<Message = Self::Message>;
    /// The state, or [`crate::message::Stateless`].
    type State: ActorState;
    /// The message family: an enum with `#[serde(tag = "type")]`.
    type Message: MessageFamily;

    /// The mandatory unhandled policy: `Fail` or `Drop`.
    const UNHANDLED: Unhandled;

    /// The maximum duration of one turn; a longer turn ends as `failed` (`timeout`). `None` uses the node default.
    ///
    /// The timeout (like a lost lease or a node stop) cancels the handler at its next `.await`. A handler blocked in
    /// code that does not yield (blocking I/O, `std::thread::sleep`) runs on until that code returns: the outcome is
    /// the same (`failed(timeout)`, never `done`), but the turn stays taken meanwhile. Run blocking calls with
    /// `tokio::task::spawn_blocking`.
    const MAX_TURN: Option<Duration> = None;

    /// Handles one message. Everything the handler does through `turn` is buffered and committed atomically when it
    /// returns `Ok`; an `Err` discards it and fails the message (retried under its retry policy unless permanent).
    fn handle(&self, turn: &mut Turn<Self>, message: Self::Message) -> impl Future<Output = HandlerResult> + Send;
}

/// An actor definition ready to be hosted by a node: the actor value, owned or shared.
pub struct ActorType<A: Actor> {
    actor: Arc<A>,
}

impl<A: Actor> ActorType<A> {
    /// The type of `actor`.
    pub fn new(actor: A) -> Self {
        Self::shared(Arc::new(actor))
    }

    /// The type of a shared `actor`.
    pub fn shared(actor: Arc<A>) -> Self {
        Self { actor }
    }

    /// The type's template, as declared by its ref.
    pub fn template(&self) -> &'static str {
        <A::Ref as ActorRef>::TEMPLATE
    }

    pub(crate) fn into_hosted(self) -> Arc<dyn HostedActor> {
        Arc::new(Hosted { actor: self.actor, analysis: analyse::<A>() })
    }
}

impl<A: Actor> From<A> for ActorType<A> {
    fn from(actor: A) -> Self {
        Self::new(actor)
    }
}

/// How a handler run ended, or what the unhandled policy decided.
pub(crate) enum RunEnd {
    Returned,
    DecidedFail(String),
    DecidedDrop,
    Threw(HandlerFailure),
}

/// What a run produced.
pub(crate) struct RunResult {
    pub end: RunEnd,
    pub effects: EffectBuffer,
    /// The encoded new state, when the handler assigned it.
    pub new_state: Option<Result<Option<Value>, String>>,
}

/// Runs a loaded turn with its info and resolver.
pub(crate) type RunTurn = Box<dyn FnOnce(Arc<TurnInfo>, Arc<TypeResolver>) -> BoxFuture<'static, RunResult> + Send>;

/// A loaded turn, ready to run.
pub(crate) struct Loaded {
    /// Why the message could not be decoded, when the unhandled policy runs.
    pub unhandled: Option<String>,
    pub run: RunTurn,
}

/// A hosted type with its generic parameters erased, as the runtime drives it.
pub(crate) trait HostedActor: Send + Sync {
    fn name(&self) -> &'static str;
    fn problems(&self) -> Vec<String>;
    fn template(&self) -> Option<&UriTemplate>;
    fn contract(&self) -> Option<&Arc<Contract>>;
    fn max_turn(&self) -> Option<Duration>;
    fn check_state(&self, state: Option<&Value>) -> Result<(), String>;
    fn check_payload(&self, message_type: &str, payload: &Value) -> Result<(), String>;
    fn canonical_problem(&self, address: &ActorAddress) -> Option<String>;
    fn load(&self, claim: &Claim) -> Result<Loaded, String>;
}

struct Analysis<A: Actor> {
    binding: RefBinding<A::Ref>,
    family: FamilyInfo,
    contract: Arc<Contract>,
}

struct Hosted<A: Actor> {
    actor: Arc<A>,
    analysis: Result<Analysis<A>, Vec<String>>,
}

/// Checks the type `A` and builds its contract; fails with every problem found.
fn analyse<A: Actor>() -> Result<Analysis<A>, Vec<String>> {
    let mut problems = RefBinding::<A::Ref>::problems().into_iter().map(|p| format!("Ref: {p}")).collect::<Vec<_>>();
    if A::MAX_TURN.is_some_and(|d| d.is_zero()) {
        problems.push("MAX_TURN must be positive".to_owned());
    }
    let binding = RefBinding::<A::Ref>::new().ok();
    let family = FamilyInfo::of::<A::Message>().map_err(|e| problems.push(format!("Messages: {e}"))).ok();
    let state = A::State::schema().map_err(|e| problems.push(format!("State schema: {e}"))).ok();
    match (binding, family, state) {
        (Some(binding), Some(family), Some(state)) if problems.is_empty() => {
            let contract = Arc::new(Contract::new(binding.template().clone(), state, family.schemas.clone()));
            Ok(Analysis { binding, family, contract })
        }
        _ => Err(problems),
    }
}

impl<A: Actor> HostedActor for Hosted<A> {
    fn name(&self) -> &'static str {
        type_name::<A>()
    }

    fn problems(&self) -> Vec<String> {
        match &self.analysis {
            Ok(_) => Vec::new(),
            Err(p) => p.clone(),
        }
    }

    fn template(&self) -> Option<&UriTemplate> {
        self.analysis.as_ref().ok().map(|a| a.binding.template())
    }

    fn contract(&self) -> Option<&Arc<Contract>> {
        self.analysis.as_ref().ok().map(|a| &a.contract)
    }

    fn max_turn(&self) -> Option<Duration> {
        A::MAX_TURN
    }

    fn check_state(&self, state: Option<&Value>) -> Result<(), String> {
        A::State::decode(state).map(|_| ())
    }

    fn check_payload(&self, message_type: &str, payload: &Value) -> Result<(), String> {
        let analysis = self.analysis.as_ref().map_err(|p| p.join("; "))?;
        if !analysis.family.accepts(message_type) {
            return Err(format!("Unknown message type '{message_type}'"));
        }
        message::decode::<A::Message>(message_type, payload).map(|_| ())
    }

    fn canonical_problem(&self, address: &ActorAddress) -> Option<String> {
        let analysis = self.analysis.as_ref().ok()?;
        let name = type_name::<A::Ref>();
        let canonical = analysis.binding.from_address(address).and_then(|r| analysis.binding.to_address(&r));
        match canonical {
            Err(e) => Some(format!("{address} is not an address of {name}: {e}")),
            Ok(c) if &c == address => None,
            Ok(c) => Some(format!("{address} is not canonical for {name}; its ref addresses {c}")),
        }
    }

    fn load(&self, claim: &Claim) -> Result<Loaded, String> {
        let analysis = self.analysis.as_ref().map_err(|p| p.join("; "))?;
        let state = A::State::decode(claim.state.as_ref()).map_err(|e| format!("State cannot be decoded: {e}"))?;
        let self_ref = analysis
            .binding
            .from_address(&claim.actor)
            .map_err(|e| format!("Address cannot be decoded as {}: {e}", type_name::<A::Ref>()))?;
        let decoded: Result<A::Message, String> = if analysis.family.accepts(&claim.message.message_type) {
            message::decode(&claim.message.message_type, &claim.message.payload)
        } else {
            Err(format!("Unknown message type '{}'", claim.message.message_type))
        };
        let unhandled = decoded.as_ref().err().cloned();
        let actor = self.actor.clone();
        Ok(Loaded {
            unhandled,
            run: Box::new(move |info, resolver| {
                Box::pin(async move {
                    let mut turn = Turn::<A>::new(info, self_ref, state, resolver);
                    let end = match decoded {
                        Ok(message) => match AssertUnwindSafe(actor.handle(&mut turn, message)).catch_unwind().await {
                            Ok(Ok(())) => RunEnd::Returned,
                            Ok(Err(e)) => RunEnd::Threw(HandlerFailure::Error(e)),
                            Err(panic) => RunEnd::Threw(HandlerFailure::Panic(panic_message(&*panic))),
                        },
                        Err(reason) => match A::UNHANDLED {
                            Unhandled::Fail => RunEnd::DecidedFail(reason),
                            Unhandled::Drop => RunEnd::DecidedDrop,
                        },
                    };
                    let (state, assigned, effects) = turn.into_parts();
                    let new_state = assigned.then(|| state.encode());
                    RunResult { end, effects, new_state }
                })
            }),
        })
    }
}

/// The message of a panic payload.
pub(crate) fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}

/// The contract of actor type `A`, as a node hosting it registers it: template, state schema and message schemas.
/// Fails with the type's problems when it is invalid (see [`crate::runtime::StartError`]).
pub fn contract_of<A: Actor>() -> Result<Contract, Vec<String>> {
    analyse::<A>().map(|a| a.contract.as_ref().clone())
}
