//! Example 9 — supervision and dead letters.
//!
//! A failed message is terminal and never blocks its mailbox. Two ready-made actors of `cicada-actors` act on it: the
//! [`DeadLetters`](cicada_actors::DeadLetters) actor, told of every failure by a watch on `*`, keeps the failed message,
//! and an operator redrives or discards it with a message; a [`Supervisor`](cicada_actors::Supervisor) restarts its
//! child when the child fails through a fault of its own. This mailer fails while its mail server is down: its
//! supervisor restarts it, and once the server is back its dead letters are redriven (see `tests/supervision.rs`). A
//! restart empties the mailbox of the child and gives it the state of its entry again. In a service:
//!
//! ```no_run
//! # use cicada::{ActorRef, ActorSystem, WatchTarget};
//! # use cicada_actors::{
//! #     Child, DeadLetters, DeadLettersActor, DeadLettersState, Supervisor, SupervisorActor, SupervisorMsg,
//! #     SupervisorState,
//! # };
//! # use cicada_examples::supervision::{Mailer, MailerActor, MailerState, Smtp};
//! # async fn run(store: cicada::Store, smtp: impl Smtp) -> Result<(), Box<dyn std::error::Error>> {
//! let system = ActorSystem::builder(store)
//!     .host(MailerActor { smtp })
//!     .host(DeadLettersActor::default())
//!     .host(SupervisorActor)
//!     .build();
//! system.start().await?;
//! system.create(&DeadLetters::default(), &DeadLettersState::default()).await?;
//! system.watch(&WatchTarget::Every, &DeadLetters::default().address()?).await?;
//! let mailer = Child::new(&Mailer { id: "m".into() }, &MailerState::default())?;
//! let supervisor = Supervisor::named("mailers");
//! system.create(&supervisor, &SupervisorState::new().child(mailer)).await?;
//! system.send(&supervisor, SupervisorMsg::Start).await?;
//! # Ok(()) }
//! ```

use std::future::Future;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

/// One mailer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailer {
    /// The mailer id.
    pub id: String,
}

impl ActorRef for Mailer {
    const TYPE: &'static str = "mailers";
    type Message = MailerMsg;
}

/// Messages of [`Mailer`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum MailerMsg {
    /// Sends one mail to `to`.
    SendMail {
        /// The recipient.
        to: String,
    },
}

/// State of a [`Mailer`]: who was mailed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MailerState {
    /// The recipients mailed so far.
    pub sent: Vec<String>,
}

/// The mail server cannot be reached.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct SmtpError(pub String);

/// The mail server.
pub trait Smtp: Send + Sync + 'static {
    /// Sends a mail; fails when the server cannot be reached.
    fn send(&self, to: &str) -> impl Future<Output = Result<(), SmtpError>> + Send;
}

/// The mailer type. A message that fails ends `failed`: it has no retry policy, so it becomes a dead letter, and the
/// failure of its handler is a fault of the mailer, which its supervisor restarts.
pub struct MailerActor<S> {
    /// The mail server.
    pub smtp: S,
}

impl<S: Smtp> Actor for MailerActor<S> {
    type Ref = Mailer;
    type State = MailerState;

    async fn handle(&self, turn: &mut Turn<Self>, message: MailerMsg) -> HandlerResult {
        match message {
            MailerMsg::SendMail { to } => {
                self.smtp.send(&to).await?;
                turn.state_mut().sent.push(to);
            }
        }
        Ok(())
    }
}
