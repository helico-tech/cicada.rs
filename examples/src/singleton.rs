//! Example 1 — a singleton that keeps no data.
//!
//! A ref without fields has exactly one instance: `maintenance.cleanup/_`. It is created once (create
//! is idempotent, so every node may do it at start) and keeps no data (`NoState`). What it adds over a plain function
//! is that its turns never overlap (G1), wherever the message comes from, and that a request survives a node crash.

use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use cicada::time::{self, Timestamp};
use serde::{Deserialize, Serialize};

/// The one and only cleanup actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cleanup;

impl ActorRef for Cleanup {
    const TYPE: &'static str = "maintenance.cleanup";
    type Message = CleanupMsg;
}

/// Messages of [`Cleanup`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum CleanupMsg {
    /// Deletes temporary files older than `retention_hours` (24 by default).
    Purge {
        /// Age limit in hours; must not be negative.
        #[serde(default = "default_retention_hours")]
        retention_hours: i32,
    },
}

fn default_retention_hours() -> i32 {
    24
}

/// The external system the singleton cleans up.
pub trait TempFiles: Send + Sync + 'static {
    /// Deletes everything older than `cutoff` and returns how many files went. Called at least once per message (G4),
    /// so it receives the message id as `idempotency_key`.
    fn purge_older_than(&self, cutoff: Timestamp, idempotency_key: &str) -> impl Future<Output = i32> + Send;
}

/// The cleanup type. Dependencies are plain fields of the actor value: a test passes a fake and the service passes
/// the real thing.
pub struct CleanupActor<F> {
    /// The files to clean up.
    pub files: F,
}

impl<F: TempFiles> Actor for CleanupActor<F> {
    type Ref = Cleanup;
    type State = NoState;
    type Message = CleanupMsg;
    const UNHANDLED: Unhandled = Unhandled::Drop;
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(10 * 60));

    async fn handle(&self, turn: &mut Turn<Self>, message: CleanupMsg) -> HandlerResult {
        match message {
            CleanupMsg::Purge { retention_hours } => {
                if retention_hours < 0 {
                    // An error ends the message `failed`.
                    return Err(HandlerError::msg("retentionHours must not be negative"));
                }
                let cutoff = time::sub(turn.now(), Duration::from_secs(retention_hours as u64 * 3600));
                self.files.purge_older_than(cutoff, turn.message().id.as_str()).await;
            }
        }
        Ok(())
    }
}
