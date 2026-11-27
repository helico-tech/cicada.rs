//! A native macOS inspector of a running Cicada system, over the HTTP API through `cicada-client` (ADR 0076).
//!
//! ```no_run
//! fn main() -> std::process::ExitCode {
//!     cicada_inspector::Inspector::new().run() // standalone, on the current context of `cicada config`
//! }
//! ```
//!
//! The window has a sidebar (Overview; Browse: Actors, Messages, Running Turns, Outcomes, Nodes, Types, Watches), a
//! toolbar with back and forward, a context pop-up and Refresh (⌘R), and the Overview: the store's counts, the health,
//! the nodes, the running turns and the types with their actor counts. The other items say "Coming soon". The server
//! comes from the contexts of `cicada config`, resolved as `cicada` resolves them. Refresh is by hand.

#[allow(dead_code)] // the window comes next
mod model;
#[cfg(test)]
mod tests;

use std::process::ExitCode;

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
        eprintln!("{}", model::NEEDS_MACOS);
        ExitCode::FAILURE
    }
}
