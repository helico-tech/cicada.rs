//! Building blocks on Cicada's public API.
//!
//! | Module | What |
//! | --- | --- |
//! | [`time`] | [`Schedule`], when a recurring job runs next |
//! | [`supervision`] | [`DeadLettersActor`], the dead-letter queue; [`FailureWatcher`], the job that fills it |

pub mod supervision;
pub mod time;

pub use supervision::{
    DeadLetter, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, FailureWatcher, WatchError,
};
pub use time::{Schedule, ScheduleError};
