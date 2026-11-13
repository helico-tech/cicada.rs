//! Example 10 — a shared rate limit.
//!
//! Many signups share one text-message gateway that takes one message a second. Each [`Signup`] creates the limiter
//! [`sms_limiter`] and submits its text to it in one turn; the
//! [`RateLimiterActor`](cicada_actors::RateLimiterActor) sends each text to the [`Sms`] sender at the next free slot,
//! five at once after idle time, and refuses one that would wait more than a minute. The limiter is the sender of what
//! it forwards, so the text carries the address to reply to in its payload, and its retry policy at the sender is
//! chosen in the submission (see `tests/rate_limit.rs`). In a service the host creates the limiter at startup:
//!
//! ```no_run
//! # use cicada::ActorSystem;
//! # use cicada_actors::RateLimiterActor;
//! # use cicada_examples::rate_limit::{Gateway, SignupActor, Sms, SmsActor, sms_limiter, sms_rate};
//! # async fn run(store: cicada::Store, gateway: impl Gateway) -> Result<(), Box<dyn std::error::Error>> {
//! let system = ActorSystem::builder(store).host(RateLimiterActor).host(SignupActor).host(SmsActor { gateway }).build();
//! system.start().await?;
//! system.create(&sms_limiter(), &sms_rate()).await?;
//! system.create(&Sms, &cicada::NoState {}).await?;
//! # Ok(()) }
//! ```

use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use cicada_actors::{RateLimiter, RateLimiterMsg, RateLimiterState};
use serde::{Deserialize, Serialize};

/// The limiter of the gateway, `cicada.rate-limiter/sms`.
pub fn sms_limiter() -> RateLimiter {
    RateLimiter::named("sms")
}

/// One text a second, five at once after idle time, and no text that waits longer than a minute.
pub fn sms_rate() -> RateLimiterState {
    RateLimiterState::new(1, Duration::from_secs(1), 5, Duration::from_secs(60)).expect("a valid rate")
}

/// One signup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signup {
    /// The signup id.
    pub id: String,
}

impl ActorRef for Signup {
    const TYPE: &'static str = "signups";
    type Message = SignupMsg;
}

/// Messages of [`Signup`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum SignupMsg {
    /// Registers `phone` and submits a welcome text to the limiter.
    Register {
        /// The phone number.
        phone: String,
    },
    /// The welcome text went out: the reply of the [`Sms`] sender.
    Texted,
}

/// State of a [`Signup`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SignupState {
    /// The registered phone number.
    pub phone: Option<String>,
    /// Whether the welcome text went out.
    pub texted: bool,
}

/// The signup type.
pub struct SignupActor;

impl Actor for SignupActor {
    type Ref = Signup;
    type State = SignupState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SignupMsg) -> HandlerResult {
        match message {
            SignupMsg::Register { phone } => {
                // Creates the limiter if it is not there yet; one that exists keeps its settings and reservations.
                turn.create(&sms_limiter(), &sms_rate())?;
                let text = SmsMsg::Send {
                    phone: phone.clone(),
                    text: "Welcome!".into(),
                    reply_to: turn.self_address().clone(),
                };
                let retry = RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5))?)?;
                turn.send(&sms_limiter(), RateLimiterMsg::submit(&Sms, text, Some(retry))?)?;
                turn.state_mut().phone = Some(phone);
            }
            SignupMsg::Texted => turn.state_mut().texted = true,
        }
        Ok(())
    }
}

/// The sender of texts, `sms/_`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sms;

impl ActorRef for Sms {
    const TYPE: &'static str = "sms";
    type Message = SmsMsg;
}

/// Messages of [`Sms`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum SmsMsg {
    /// Sends `text` to `phone` and tells `reply_to`: the sender of this message is the limiter, not the signup.
    Send {
        /// The phone number.
        phone: String,
        /// The text.
        text: String,
        /// The signup to tell.
        reply_to: ActorAddress,
    },
}

/// The gateway cannot be reached.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct GatewayError(pub String);

/// The text-message gateway.
pub trait Gateway: Send + Sync + 'static {
    /// Sends a text; fails when the gateway cannot be reached.
    fn send(&self, phone: &str, text: &str) -> impl Future<Output = Result<(), GatewayError>> + Send;
}

/// The sender type. A send that fails is tried again by the retry policy the submission chose, without passing the
/// limiter again.
pub struct SmsActor<G> {
    /// The gateway.
    pub gateway: G,
}

impl<G: Gateway> Actor for SmsActor<G> {
    type Ref = Sms;
    type State = NoState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SmsMsg) -> HandlerResult {
        let SmsMsg::Send { phone, text, reply_to } = message;
        self.gateway.send(&phone, &text).await?;
        turn.send_to(&reply_to, &SignupMsg::Texted, SendOptions::new()).await?;
        Ok(())
    }
}
