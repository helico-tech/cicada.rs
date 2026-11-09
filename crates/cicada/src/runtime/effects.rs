//! Buffered effects of a turn.

use crate::store::{ActorCreate, MessageId, OutgoingMessage};

/// Buffered effects of a turn, applied by its commit.
#[derive(Debug, Default)]
pub(crate) struct EffectBuffer {
    pub creates: Vec<ActorCreate>,
    pub sends: Vec<OutgoingMessage>,
    /// Pending messages of this instance to cancel at commit.
    pub cancels: Vec<MessageId>,
    pub stop: bool,
}
