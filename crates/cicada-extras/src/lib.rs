//! Building blocks on Cicada's public API.
//!
//! | Module | What |
//! | --- | --- |
//! | [`time`] | [`Schedule`], when a recurring job runs next |
//! | [`guard`] | [`CrashLoopGuard`], a plugin that fails messages whose claims keep ending without a commit |
//! | [`supervision`] | [`DeadLettersActor`], the dead-letter queue; [`FailureWatcher`], the job that fills it |

pub mod guard;
pub mod supervision;
pub mod time;

pub use guard::{CrashLoopError, CrashLoopGuard};
pub use supervision::{
    DeadLetter, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, FailureWatcher, WatchError,
};
pub use time::{Schedule, ScheduleError};
