//! Worked examples of Cicada actor types, each with tests (under `tests/`, run with `TestActors` on virtual time).
//!
//! | Module | Pattern |
//! | --- | --- |
//! | [`singleton`] | Example 1: a singleton that keeps no data (`maintenance.cleanup/_`) whose turns never overlap. |
//! | [`timer`] | Example 2: a timer / timeout with a generation number; a stale timeout is ignored. |
//! | [`cron`] | Example 3: a cron job; each tick does the work and arms the next one in the same commit. |
//! | [`reply`] | Example 4: request and reply; the responder replies to `message.from` with the correlation id. |
//! | [`saga`] | Example 5: an orchestrator saga with compensation, and per-message retry set by the sender. |
//! | [`steps`] | Example 6: durable steps; every step is its own turn, so a committed step never runs again. |
//! | [`fanout`] | Example 7: fan-out and fan-in; one child per item, the parent counts the replies. |
//! | [`registry`] | Example 8: a registry actor that keeps the refs of the instances it created. |
//! | [`supervision`] | Example 9: a supervisor restarts a child at fault; dead letters keep what failed, to redrive. |
//! | [`rate_limit`] | Example 10: a shared rate limit; signups create a limiter and submit their texts to it in one turn. |
//! | [`debounce`] | Example 11: a burst of edits, one reindex; each edit submits the latest version to a debouncer. |
//! | [`batching`] | Example 12: audit events in bulk; each change adds its event to a batcher that writes 50 at once. |
//! | [`flow_onboarding`] | Example 13: the onboarding of example 6 as a durable flow; async code, one step per turn, a durable sleep. |
//! | [`flow_events`] | Example 14: flows started by an event; a placed order starts a receipt and a fulfilment in one commit, a batcher a digest. |
//! | [`remote_worker`] | Example 15: the cleanup singleton of example 1, unchanged, in a worker without database access that leases its turns over HTTP. |
//! | [`dead_letters_upgrade`] | Example 16: the upgrade of dead letters to 0.17.0; export, convert, reset and restore around the new contract. |
//! | [`flow_approval`] | Example 17: a purchase flow that requests a person's approval and waits for the reply for at most a week, with no handler open, then expires; a sign-off that joins the approvals of several teams, with partial results at the deadline; and the first of several quotes. |
//! | [`flow_resources`] | Example 18: a job that owns a sandbox; cancelled, its cleanup stops the sandbox, waits for the removal and releases the capacity by the reservation's key. |
//! | [`heartbeat`] | Example 19: a nightly import that did not run; each file beats a heartbeat watch, which alerts once after 25 quiet hours. |
//! | [`websocket_session`] | Example 20: a websocket session; a host follows the session's durable frames, writes them to its socket and forwards the client's acknowledgements, so any host can serve a reconnect. |
//! | [`sync`] | The architecture's example: a paged import, one turn per page, rescheduled hourly. |
//!
//! Every example has the same shape: a public ref struct per actor type (its one field is the key), a
//! message enum with `#[serde(tag = "type")]`, a state struct, and an actor struct whose fields are its dependencies
//! (external services, faked in the tests), with one `impl Actor` whose `handle` matches on the message. A flow
//! ([`flow_onboarding`]) is the exception: its code is a [`Flow`](cicada_actors::Flow), and its ref and state are the
//! runner's.

pub mod batching;
pub mod cron;
pub mod dead_letters_upgrade;
pub mod debounce;
pub mod fanout;
pub mod flow_approval;
pub mod flow_events;
pub mod flow_onboarding;
pub mod flow_resources;
pub mod heartbeat;
pub mod rate_limit;
pub mod registry;
pub mod remote_worker;
pub mod reply;
pub mod saga;
pub mod singleton;
pub mod steps;
pub mod supervision;
pub mod sync;
pub mod timer;
pub mod websocket_session;

/// The Rust code blocks of the README, compiled (and, unless `no_run`, run) by `cargo test -p cicada-examples --doc`.
#[cfg(doctest)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/README.md"))]
pub struct ReadmeDoctests;

/// The Rust code blocks of the guide of the core (`docs/guide/core.md`), compiled and run like the README's.
#[cfg(doctest)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/guide-core.md"))]
pub struct CoreGuideDoctests;

/// The Rust code blocks of the guide of HTTP and the client (`docs/guide/http.md`), compiled and run like the README's.
#[cfg(doctest)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/guide-http.md"))]
pub struct HttpGuideDoctests;

/// The Rust code blocks of the guide of the app (`docs/guide/app.md`), compiled and run like the README's.
#[cfg(doctest)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/guide-app.md"))]
pub struct AppGuideDoctests;

/// The Rust code blocks of the guide of the ready-made actors (`docs/guide/actors.md`), compiled and run like the
/// README's.
#[cfg(doctest)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/guide-actors.md"))]
pub struct ActorsGuideDoctests;

/// The Rust code blocks of the design guide (`docs/guide/design.md`), compiled and run like the README's.
#[cfg(doctest)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/guide-design.md"))]
pub struct DesignGuideDoctests;

/// The Rust code blocks of the operations guide, compiled and run like the README's.
#[cfg(doctest)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/operations.md"))]
pub struct OperationsDoctests;

/// The Rust blocks of the upgrade procedure, compiled without running destructive operations.
#[cfg(doctest)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/upgrading.md"))]
pub struct UpgradeGuideDoctests;
