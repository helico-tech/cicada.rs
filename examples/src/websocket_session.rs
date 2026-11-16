//! Example 20 — a websocket session: durable output for a socket that a host holds.
//!
//! A socket lives in one process and dies with it; an actor's state survives both. The [`SessionActor`] keeps what the
//! client has not yet processed: at most [`MAX_FRAMES`] frames, each numbered with the next sequence, and the
//! client's cumulative acknowledgement. A host that owns the socket follows that state with public `ActorSystem`
//! reads and the change feed ([`bridge`]), writes the frames, and forwards the client's `Ack` as an ordinary message.
//! Any host that shares the store can serve a reconnect, so the actor needs no pin and the host keeps no registry of
//! sockets. [`adapter`] puts the bridge behind an axum websocket route.
//!
//! The protocol is this example's application convention, not a Cicada guarantee:
//!
//! - **One logical client per session.** The host creates the session before the first socket, under a key that is
//!   never used again: one key is one stream life. Independent tabs or devices get their own keys. Deleting the actor
//!   ends the promise of replay for what it held; a new stream needs a new key, never a reset at the old one. Nothing
//!   expires on its own, and the buffer bound limits neither the mailbox nor the number of sessions.
//! - **Accept or refuse, never drop.** [`SessionMsg::Enqueue`] carries the body and the producer's typed reply
//!   address. The frame, the reply and the completion of the message commit together: [`ProducerMsg::Accepted`] names
//!   the Enqueue's message id and the frame's sequence. Otherwise [`ProducerMsg::Rejected`] names one [`Refusal`], in
//!   this order: an oversized body, a full buffer, an exhausted sequence; nothing changes and nothing is retried. A
//!   reply actor that no longer exists rejects the commit like any send to an absent actor: no frame is kept.
//! - **NUL is refused before the session sees it.** Cicada refuses a NUL character in a payload when it is sent:
//!   `ActorSystem::send` returns `OpError::Invalid` and `Turn::send` returns `TurnError::Invalid`, with no message
//!   queued and so no reply. Only a storable body longer than [`MAX_BODY_BYTES`] gets the session's
//!   [`Refusal::InvalidBody`].
//! - **A sent message is not an accepted frame.** Only the reply says the session accepted a body. A producer may send
//!   a new message after a refusal. When it cannot know whether its send was stored, it sends again with the same
//!   message id within Cicada's deduplication window; the example promises no deduplication of business requests
//!   beyond that.
//! - **Only the client trims.** [`SessionMsg::Ack`] asserts that the client processed every frame up to `through`.
//!   It advances the acknowledgement and drops those frames in one turn; a repeated, older, larger than assigned,
//!   non-positive or other stream's ack changes nothing. A socket write is never evidence of processing.

use std::collections::VecDeque;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

/// The most frames a session keeps that the client has not acknowledged.
pub const MAX_FRAMES: usize = 100;

/// The most UTF-8 bytes of one frame's body.
pub const MAX_BODY_BYTES: usize = 1024;

/// The highest sequence: 2^53 − 1, the largest integer a browser's JSON numbers hold exactly.
pub const MAX_SEQ: u64 = 9_007_199_254_740_991;

/// The output stream of one logical client, for one life: its key is never used again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    /// The stream key, also the `stream` field of every socket message.
    pub stream: String,
}

impl ActorRef for Session {
    const TYPE: &'static str = "ws.sessions";
    type Message = SessionMsg;
}

/// Messages of [`Session`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum SessionMsg {
    /// Appends `body` as the next frame, or refuses it; the answer goes to `reply_to`.
    Enqueue {
        /// The frame's text.
        body: String,
        /// Who gets [`ProducerMsg::Accepted`] or [`ProducerMsg::Rejected`].
        reply_to: Producer,
    },
    /// The client processed every frame up to `through`; sent by the host for the client's socket message.
    Ack {
        /// The stream the client named.
        stream: String,
        /// The highest sequence the client processed without a gap.
        through: i64,
    },
}

/// A retained frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Frame {
    /// Its sequence, from 1.
    pub seq: u64,
    /// Its text.
    pub body: String,
}

/// State of a [`Session`]. The host creates it with the default: nothing assigned, nothing acknowledged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SessionState {
    /// The highest sequence assigned so far; 0 before the first frame.
    pub assigned: u64,
    /// The client's cumulative acknowledgement; 0 before the first.
    pub acked: u64,
    /// The frames above `acked`, in sequence; at most [`MAX_FRAMES`].
    pub frames: VecDeque<Frame>,
}

impl SessionState {
    /// Why `body` cannot be appended now, in the order of precedence; `None` when it can.
    pub fn refusal(&self, body: &str) -> Option<Refusal> {
        if body.len() > MAX_BODY_BYTES {
            Some(Refusal::InvalidBody)
        } else if self.frames.len() >= MAX_FRAMES {
            Some(Refusal::BufferFull)
        } else if self.assigned >= MAX_SEQ {
            Some(Refusal::SequenceExhausted)
        } else {
            None
        }
    }

    /// The sequence `through` when it acknowledges more than before: positive, above `acked` and at most `assigned`
    /// (so within [`MAX_SEQ`]).
    pub fn new_ack(&self, through: i64) -> Option<u64> {
        let through = u64::try_from(through).ok()?;
        (through > self.acked && through <= self.assigned).then_some(through)
    }
}

/// Why a session refused a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Refusal {
    /// The body is longer than [`MAX_BODY_BYTES`].
    InvalidBody,
    /// The session keeps [`MAX_FRAMES`] unacknowledged frames.
    BufferFull,
    /// The sequence reached [`MAX_SEQ`]; the stream needs a new session.
    SequenceExhausted,
}

/// The session type.
pub struct SessionActor;

impl Actor for SessionActor {
    type Ref = Session;
    type State = SessionState;

    async fn handle(&self, turn: &mut Turn<Self>, message: SessionMsg) -> HandlerResult {
        match message {
            SessionMsg::Enqueue { body, reply_to } => {
                let request = turn.message().id.clone();
                let reply = match turn.state().refusal(&body) {
                    Some(reason) => ProducerMsg::Rejected { request, reason },
                    None => {
                        let state = turn.state_mut();
                        state.assigned += 1;
                        let seq = state.assigned;
                        state.frames.push_back(Frame { seq, body });
                        ProducerMsg::Accepted { request, seq }
                    }
                };
                // Commits with the frame: a reply actor that is gone rejects the commit, and no frame is kept.
                turn.send(&reply_to, reply)?;
            }
            SessionMsg::Ack { stream, through } => {
                if stream != turn.self_ref().stream {
                    return Ok(());
                }
                let Some(through) = turn.state().new_ack(through) else { return Ok(()) };
                let state = turn.state_mut();
                state.frames.retain(|frame| frame.seq > through);
                state.acked = through;
            }
        }
        Ok(())
    }
}

/// A producer of output for one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Producer {
    /// The producer id.
    pub id: String,
}

impl ActorRef for Producer {
    const TYPE: &'static str = "ws.producers";
    type Message = ProducerMsg;
}

/// Messages of [`Producer`]: its command and the session's replies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum ProducerMsg {
    /// Offers `body` to the session; refused while a request is pending.
    Emit {
        /// The frame's text.
        body: String,
    },
    /// The session appended the body of `request` as frame `seq`.
    Accepted {
        /// The id of the Enqueue message.
        request: MessageId,
        /// The frame's sequence.
        seq: u64,
    },
    /// The session refused the body of `request`.
    Rejected {
        /// The id of the Enqueue message.
        request: MessageId,
        /// Why.
        reason: Refusal,
    },
}

/// A request the session has not answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Pending {
    /// The id of the Enqueue message.
    pub request: MessageId,
    /// The body offered.
    pub body: String,
}

/// The session's answer to the last request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Answer {
    /// Appended as frame `seq`.
    Accepted {
        /// The frame's sequence.
        seq: u64,
    },
    /// Refused.
    Rejected {
        /// Why.
        reason: Refusal,
    },
}

/// State of a [`Producer`]: at most one pending request and the last answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProducerState {
    /// The session it writes to.
    pub session: Session,
    /// The request in flight.
    pub pending: Option<Pending>,
    /// The answer to the last request.
    pub last: Option<Answer>,
}

impl ProducerState {
    /// A producer for `session` with nothing in flight.
    pub fn new(session: Session) -> Self {
        Self { session, pending: None, last: None }
    }
}

/// The producer type.
pub struct ProducerActor;

impl Actor for ProducerActor {
    type Ref = Producer;
    type State = ProducerState;

    async fn handle(&self, turn: &mut Turn<Self>, message: ProducerMsg) -> HandlerResult {
        let (request, answer) = match message {
            ProducerMsg::Emit { body } => {
                if turn.state().pending.is_some() {
                    return Err(turn.fail("A request is pending; emit again after its answer"));
                }
                let session = turn.state().session.clone();
                let reply_to = turn.self_ref().clone();
                let request = turn.send(&session, SessionMsg::Enqueue { body: body.clone(), reply_to })?;
                turn.state_mut().pending = Some(Pending { request, body });
                return Ok(());
            }
            ProducerMsg::Accepted { request, seq } => (request, Answer::Accepted { seq }),
            ProducerMsg::Rejected { request, reason } => (request, Answer::Rejected { reason }),
        };
        // An answer to another request (unsolicited, or to a host's own send) is ignored.
        if turn.state().pending.as_ref().is_some_and(|p| p.request == request) {
            let state = turn.state_mut();
            state.pending = None;
            state.last = Some(answer);
        }
        Ok(())
    }
}
