//! The crash-loop guard.

use async_trait::async_trait;
use cicada::plugin::{ClaimContext, HookResult, Plugin};

/// Ends a message `failed` when its earlier claims ended without a commit too often: the sign of a message that kills
/// its node (out of memory, `std::process::exit`, a native crash, an abort on panic), which would otherwise be
/// claimed again forever.
///
/// A message's `attempt` counts its claims and `failures` its failed runs that were committed, so
/// `attempt - failures - 1` claims before this one never reached a commit. When that reaches the limit, the guard's
/// `on_claim` hook fails: the handler does not run and the message ends `failed` with reason
/// `onClaim of plugin CrashLoopGuard failed: crash loop: N claims without a commit`. It is never retried, and shows up
/// under `/failures`.
///
/// Every claim without a commit counts, whatever ended it: a lease lost to a long pause or a partition, and a lease
/// released by a node that stopped before the turn finished, count as much as a crash. Keep the limit well above the
/// number of such interruptions a message may see in normal operation (rolling deploys during long turns).
///
/// ```
/// # use std::sync::Arc;
/// # fn node(store: Arc<dyn cicada::store::Store>) -> Result<(), String> {
/// use cicada::ActorSystem;
/// use cicada_extras::CrashLoopGuard;
///
/// let system = ActorSystem::builder(store)
///     .install(CrashLoopGuard::new(5)?) // on the node, for every hosted type
///     .build();
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrashLoopGuard {
    max_uncommitted_claims: u32,
}

impl CrashLoopGuard {
    /// The default limit.
    pub const DEFAULT_MAX_UNCOMMITTED_CLAIMS: u32 = 5;

    /// A guard that fails a message once `max_uncommitted_claims` earlier claims of it ended without a commit. At
    /// least 1.
    pub fn new(max_uncommitted_claims: u32) -> Result<Self, String> {
        if max_uncommitted_claims < 1 {
            return Err(format!("max_uncommitted_claims must be at least 1, was {max_uncommitted_claims}"));
        }
        Ok(Self { max_uncommitted_claims })
    }

    /// The limit.
    pub fn max_uncommitted_claims(&self) -> u32 {
        self.max_uncommitted_claims
    }
}

impl Default for CrashLoopGuard {
    /// A guard with the default limit of 5.
    fn default() -> Self {
        Self { max_uncommitted_claims: Self::DEFAULT_MAX_UNCOMMITTED_CLAIMS }
    }
}

#[async_trait]
impl Plugin for CrashLoopGuard {
    fn name(&self) -> &str {
        "CrashLoopGuard"
    }

    async fn on_claim(&self, ctx: &mut ClaimContext<'_>) -> HookResult {
        let message = ctx.info().message();
        let uncommitted = message.attempt.saturating_sub(message.failures).saturating_sub(1);
        if uncommitted >= self.max_uncommitted_claims {
            return Err(CrashLoopError { claims: uncommitted }.into());
        }
        Ok(())
    }
}

/// The error of [`CrashLoopGuard`] that fails a message that was claimed `claims` times without a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("crash loop: {claims} claims without a commit")]
pub struct CrashLoopError {
    /// Earlier claims of the message that ended without a commit.
    pub claims: u32,
}
