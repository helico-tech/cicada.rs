//! Worked examples of Cicada actor types, each with tests (under `tests/`, run with `TestActors` on virtual time).
//!
//! | Module | Pattern |
//! | --- | --- |
//! | [`singleton`] | Example 1: a stateless singleton (`actor://maintenance/cleanup`) whose turns never overlap. |
//! | [`timer`] | Example 2: a timer / timeout with a generation number; a stale timeout is ignored. |
//! | [`cron`] | Example 3: a cron job; each tick does the work and arms the next one in the same commit. |
//! | [`reply`] | Example 4: request and reply; the responder replies to `message.from` with the correlation id. |
//! | [`saga`] | Example 5: an orchestrator saga with compensation, and per-message retry set by the sender. |
//! | [`steps`] | Example 6: durable steps; every step is its own turn, so a committed step never runs again. |
//! | [`fanout`] | Example 7: fan-out and fan-in; one child per item, the parent counts the replies. |
//! | [`registry`] | Example 8: a registry actor that keeps the refs of the instances it created. |
//! | [`supervision`] | Example 9: supervision; failed messages become dead letters that an operator redrives. |
//! | [`sync`] | The architecture's example: a paged import, one turn per page, rescheduled hourly. |
//!
//! Every example has the same shape: a public ref struct per actor type (its fields are the template parameters), a
//! message enum with `#[serde(tag = "type")]`, a state struct, and an actor struct whose fields are its dependencies
//! (external services, faked in the tests), with one `impl Actor` whose `handle` matches on the message.

pub mod cron;
pub mod fanout;
pub mod registry;
pub mod reply;
pub mod saga;
pub mod singleton;
pub mod steps;
pub mod supervision;
pub mod sync;
pub mod timer;
