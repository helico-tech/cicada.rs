//! Example 9 — supervision and dead letters.
//!
//! A failed message is terminal and never blocks its mailbox; the store keeps it in the failures list. Supervision is
//! built on that list with `cicada-extras`: a [`FailureWatcher`](cicada_extras::FailureWatcher) job copies every
//! failure into the [`DeadLetters`](cicada_extras::DeadLetters) actor, where an operator redrives or discards it with a
//! message. This mailer fails while its mail server is down; once the server is back, its dead letters are redriven
//! (see `tests/supervision.rs`). In a service:
//!
//! ```no_run
//! # use std::{sync::Arc, time::Duration};
//! # use cicada::ActorSystem;
//! # use cicada_examples::supervision::{MailerActor, Smtp};
//! # use cicada_extras::{DeadLettersActor, FailureWatcher};
//! # async fn run(store: Arc<dyn cicada::store::Store>, smtp: impl Smtp) -> Result<(), cicada::StartError> {
//! let system = ActorSystem::builder(store).host(MailerActor { smtp }).host(DeadLettersActor::default()).build();
//! system.start().await?;
//! let watcher = FailureWatcher::new(system.clone()).start(Duration::from_secs(10)); // abort() it on shutdown
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

/// The mailer type. A message that fails ends `failed`: it has no retry policy, so it becomes a dead letter.
pub struct MailerActor<S> {
    /// The mail server.
    pub smtp: S,
}

impl<S: Smtp> Actor for MailerActor<S> {
    type Ref = Mailer;
    type State = MailerState;
    type Message = MailerMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

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
