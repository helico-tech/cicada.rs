//! A native macOS inspector of a running Cicada system, over the HTTP API through `cicada-client` (ADR 0076).
//!
//! ```no_run
//! fn main() -> std::process::ExitCode {
//!     cicada_inspector::Inspector::new().run() // standalone, on the current context of `cicada config`
//! }
//! ```
//!
//! The window has a sidebar (Overview; Browse: Actors, Messages, Running Turns, Outcomes, Nodes, Types, Watches), a
//! toolbar with back and forward, a search field, a context pop-up, Refresh (⌘R) and a toggle of the pane, and the
//! Overview: the store's counts, the health, the nodes, the running turns and the types with their actor counts.
//! Actors lists the actors under the searched address prefix, 100 at a time as the table scrolls, with their type,
//! version, created time and status (running from one sample of the first 200 running turns, paused from the actor);
//! the pane shows the selected actor's details, running turn, state as highlighted JSON (cut at 256 KiB), mailbox and
//! watches, read only. Messages lists the pending messages to the actors under the searched prefix, 100 at a time,
//! each once, with their id, target, type, delivery time and attempt; the pane shows the selected message's envelope
//! and payload as highlighted JSON, and Show Actor selects its actor on the Actors page, recorded in back and forward.
//! Outcomes lists the finished messages under the prefix by status (Failed by default, Cancelled, Done or All), 100
//! at a time, each once, and the sidebar shows the store's failed count; the pane shows the selected outcome's error,
//! attempts, retry policy, envelope and payload, the message recorded as its cause and those that record it as theirs,
//! each a link recorded in back and forward. Running Turns lists the turns running under the prefix, 100 at a time,
//! with their actor, message, node, lease end and a cancel request; the pane shows the selected turn with Show Actor,
//! and back to it reads the actor's turn again, marking one that ended "no longer running". Nodes lists every node
//! with its status and last heartbeat as the server gave them, read on the first visit, on the page's Refresh and
//! after a context switch, and filtered by id prefix; the pane reads the selected node with its running turns, each
//! with Show Actor. Types lists registered contracts and displays one schema at a time. Watches lists the stored
//! watches, with a filter over the loaded rows and Show Actor for the watcher and any single-actor target. Every page
//! is read only. The server comes from the contexts of `cicada config`, resolved as `cicada` resolves them. Refresh is
//! by hand.

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod actors;
#[cfg(target_os = "macos")]
mod appkit;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod json;
#[cfg(any(target_os = "macos", test))]
mod lifecycle;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod messages;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod model;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod nodes;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod outcomes;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod pager;
#[cfg(test)]
mod tests;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod turns;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod types;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod watches;

use std::process::ExitCode;

#[cfg(target_os = "macos")]
pub use appkit::Embedded;

/// The inspector window as an app of its own: [`Self::run`] owns the main thread until the window closes.
#[derive(Debug, Default)]
pub struct Inspector {
    _private: (),
}

impl Inspector {
    /// An inspector on the process environment and the config file it names.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the window on the current context and runs the app until the window closes or the app quits. Call this
    /// on the main thread on macOS 14 or later; it has a Dock icon and a main menu with Quit, Close (⌘W) and Refresh
    /// (⌘R). Elsewhere, or when startup fails, prints an error and returns exit code 1.
    pub fn run(self) -> ExitCode {
        #[cfg(target_os = "macos")]
        {
            appkit::run(environment())
        }
        #[cfg(not(target_os = "macos"))]
        {
            eprintln!("{}", model::NEEDS_MACOS);
            ExitCode::FAILURE
        }
    }
}

/// The process environment, without the variables whose name or value is not UTF-8.
#[cfg(target_os = "macos")]
fn environment() -> cicada_app::Env {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}
