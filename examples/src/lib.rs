//! Worked examples of Cicada actor types, each with tests (under `tests/`, run with `TestActors` on virtual time).
//!
//! | Module | Pattern |
//! | --- | --- |
//! | [`singleton`] | Example 1: a stateless singleton (`actor://maintenance/cleanup`) whose turns never overlap. |
//! | [`reply`] | Example 4: request and reply; the responder replies to `message.from` with the correlation id. |
//! | [`saga`] | Example 5: an orchestrator saga with compensation, and per-message retry set by the sender. |
//! | [`steps`] | Example 6: durable steps; every step is its own turn, so a committed step never runs again. |
//! | [`fanout`] | Example 7: fan-out and fan-in; one child per item, the parent counts the replies. |
//! | [`registry`] | Example 8: a registry actor that keeps the refs of the instances it created. |
//! | [`sync`] | The architecture's example: a paged import, one turn per page, rescheduled hourly. |
//!
//! Examples 2 (timer), 3 (cron) and 9 (supervision) build on `cicada-extras` and follow later.
//!
//! Every example has the same shape: a public ref struct per actor type (its fields are the template parameters), a
//! message enum with `#[serde(tag = "type")]`, a state struct, and an actor struct whose fields are its dependencies
//! (external services, faked in the tests), with one `impl Actor` whose `handle` matches on the message.

pub mod fanout;
pub mod registry;
pub mod reply;
pub mod saga;
pub mod singleton;
pub mod steps;
pub mod sync;
