//! Supervision and dead letters.
//!
//! A failed message is terminal and never blocks its mailbox; the store keeps it, with its envelope and error, in the
//! failures list (`GET /failures`) for the outcome retention. Supervision is built on that list. Handlers cannot read
//! it, so the reader is a job in the service, the [`FailureWatcher`]. It copies each failure into the
//! [`DeadLetters`] actor, which keeps the dead letters durable beyond the outcome retention and lets an operator act on
//! them as messages: `Redrive` sends the failed message again (to the same actor, same type and payload), `Discard`
//! forgets it. Host a [`DeadLettersActor`] on a node and run a [`FailureWatcher`] next to it.

mod dead_letters;
mod failure_watcher;

pub use dead_letters::{DeadLetter, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState};
pub use failure_watcher::FailureWatcher;
