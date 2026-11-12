//! Ready-made actors on Cicada's public API (ADR 0018, ADR 0028), each under a fixed type name, and a schedule helper.
//!
//! | Item | Type name | What |
//! | --- | --- | --- |
//! | [`SupervisorActor`] | `cicada.supervisor` | Starts its children, watches them, and restarts one that is at fault |
//! | [`StoredMessage`] | | A message an actor keeps to send later: a type name and a JSON payload, built from a typed ref |
//! | [`Schedule`] | | When a recurring job runs next |
//!
//! A message an actor of this crate keeps is a [`StoredMessage`]: the actor knows where it goes, so it holds no
//! address. Its typed helper checks the message against the family of a ref when the code is compiled.

mod error;
mod schedule;
mod stored;
pub mod supervisor;

pub use error::EncodeError;
pub use schedule::{Schedule, ScheduleError};
pub use stored::StoredMessage;
pub use supervisor::{Child, ChildEntry, Supervisor, SupervisorActor, SupervisorMsg, SupervisorState};
