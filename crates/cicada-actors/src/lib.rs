//! Ready-made actors on Cicada's public API (ADR 0018, ADR 0028), each under a fixed type name, a schedule helper, and
//! durable flows under a type name the host chooses (ADR 0031).
//!
//! | Item | Type name | What |
//! | --- | --- | --- |
//! | [`SupervisorActor`] | `cicada.supervisor` | Starts its children, watches them, and restarts one that is at fault |
//! | [`DeadLettersActor`] | `cicada.dead-letters` | Keeps failed messages, told by a watch; redrives or discards one |
//! | [`TopicActor`] | `cicada.topic` | Sends each published message to its subscribers, each in a turn of its own |
//! | [`SchedulerActor`] | `cicada.scheduler` | Sends a stored message to a target on each slot of a [`Schedule`] |
//! | [`RateLimiterActor`] | `cicada.rate-limiter` | Sends each submitted message to its target at the next free slot of a rate |
//! | [`DebouncerActor`] | `cicada.debouncer` | Keeps the latest submission of a burst and sends it once the burst is quiet |
//! | [`BatcherActor`] | `cicada.batcher` | Collects items and sends them to its target as one batch on count or time |
//! | [`EventActor`] | `cicada.event` | Creates and starts one run of each subscribed flow for each publication, in one commit |
//! | [`FlowActor`] | chosen by the [`Flow`] | Runs a multi-step job written as async code, one step or durable sleep per turn, replaying saved results |
//! | [`StoredMessage`] | | A message an actor keeps to send later: a type name and a JSON payload, built from a typed ref |
//! | [`Schedule`] | | When a recurring job runs next |
//!
//! A message an actor of this crate keeps is a [`StoredMessage`]: the actor knows where it goes, so it holds no
//! address. Its typed helper checks the message against the family of a ref when the code is compiled.

pub mod batcher;
pub mod dead_letters;
pub mod debouncer;
mod error;
pub mod event;
pub mod flow;
pub mod rate_limiter;
mod schedule;
pub mod scheduler;
mod stored;
pub mod supervisor;
pub mod topic;

pub use batcher::{Batcher, BatcherActor, BatcherMsg, BatcherState, InvalidBatch};
pub use dead_letters::{
    CapacityError, DeadLetter, DeadLetters, DeadLettersActor, DeadLettersMsg, DeadLettersState, LetterPayload,
};
pub use debouncer::{DebouncedMessage, Debouncer, DebouncerActor, DebouncerMsg, DebouncerState, InvalidDebounce};
pub use error::EncodeError;
pub use event::{Event, EventActor, EventMsg, EventState};
pub use flow::{
    Cleanup, CreateKind, Flow, FlowActor, FlowMsg, FlowReply, FlowRun, FlowState, FlowStatus, JournalEntry, MainReply,
    ReplyTo, Request, Step, Target, WaitMode, WaitOutcome, Waited,
};
pub use rate_limiter::{InvalidRate, RateLimiter, RateLimiterActor, RateLimiterMsg, RateLimiterState};
pub use schedule::{Schedule, ScheduleError};
pub use scheduler::{Scheduler, SchedulerActor, SchedulerMsg, SchedulerState};
pub use stored::StoredMessage;
pub use supervisor::{Child, ChildEntry, Supervisor, SupervisorActor, SupervisorMsg, SupervisorState};
pub use topic::{Topic, TopicActor, TopicMsg, TopicState};
