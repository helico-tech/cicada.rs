//! Building blocks on Cicada's public API.
//!
//! | Module | What |
//! | --- | --- |
//! | [`time`] | [`Timer`], a cancellable timeout kept in state; [`Schedule`], when a recurring job runs next |
//! | [`guard`] | [`CrashLoopGuard`], a plugin that fails messages whose claims keep ending without a commit |
//! | [`supervision`] | [`DeadLettersActor`], the dead-letter queue; [`FailureWatcher`], the job that fills it |

pub mod guard;
pub mod supervision;
pub mod time;

pub use guard::{CrashLoopError, CrashLoopGuard};
pub use supervision::{
    ActorOperations, DeadLetter, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, Failure,
    FailureWatcher, OperationError, Sent,
};
pub use time::{Schedule, ScheduleError, Timer};
