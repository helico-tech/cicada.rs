//! Buffered effects of a turn.

use crate::store::{ActorCreate, Control, MessageId, OutgoingMessage};
use crate::watch::WatchChange;

/// Buffered effects of a turn, applied by its commit.
#[derive(Debug, Default)]
pub(crate) struct EffectBuffer {
    pub creates: Vec<ActorCreate>,
    pub sends: Vec<OutgoingMessage>,
    /// Pending messages of this instance to cancel at commit.
    pub cancels: Vec<MessageId>,
    /// The watches this instance sets and removes, in the order of the calls.
    pub watches: Vec<WatchChange>,
    /// The control effects on other actors, and on this one: each effect once per actor, and of a pause and a resume
    /// of one actor only the last.
    pub controls: Vec<Control>,
    pub stop: bool,
}
