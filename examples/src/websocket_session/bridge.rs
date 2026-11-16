//! The host side of a session: one socket, served from the session's durable state.
//!
//! [`serve`] subscribes to the session's change hints before its first read, and reads again on a hint for exactly
//! this address, on a `Resync` and every [`POLL_INTERVAL`], so missed hints, another writer's store and a reconnect to
//! another host all end in a fresh read. The first read does not wait for the feed's first `Resync`. Each read
//! writes `Hello` once, then the retained frames above the read's acknowledgement that this socket has not had yet;
//! a reconnect starts again from the durable acknowledgement, so frames may repeat. The client's `Ack` goes to the
//! session as an ordinary message; nothing here trims or remembers state beyond one socket.
//!
//! The reader of the client's messages and the writer run side by side in the caller's task. Socket writes and store
//! operations are bounded by [`IO_TIMEOUT`]. A missing session, a failed or uncertain store operation, an ended
//! feed, a socket error, invalid input, the client's close and shutdown each end both halves before [`serve`]
//! returns, and leave the durable state as it was: a reconnect replays what was not acknowledged.

use std::convert::Infallible;
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

use cicada::{ActorChange, ActorRef, ActorSystem, SendResult};
use futures::stream::BoxStream;
use futures::{Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::time::{MissedTickBehavior, timeout};

use super::{Session, SessionMsg, SessionState};

/// How often a connection reads its session without a hint.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The bound of one socket write and of one store read or send.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// A message the host writes to the socket, as JSON text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// The first message of a socket: the stream and its durable acknowledgement.
    Hello {
        /// The stream key.
        stream: String,
        /// The highest sequence the session holds as processed.
        acked: u64,
    },
    /// A retained frame.
    Frame {
        /// The stream key.
        stream: String,
        /// The frame's sequence.
        seq: u64,
        /// The frame's text.
        body: String,
    },
}

/// A message the client writes to the socket, as JSON text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// The client processed every frame of `stream` up to `through`, without a gap.
    Ack {
        /// The stream key.
        stream: String,
        /// The highest sequence processed.
        through: i64,
    },
}

/// What a socket delivered to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    /// A text message.
    Text(String),
    /// A binary message, which this protocol does not use.
    Binary,
}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    /// The client closed the socket.
    ClientClosed,
    /// The host shuts down.
    Shutdown,
    /// The session does not exist (never created, or deleted).
    SessionMissing,
    /// The session holds fewer sequences than this socket was sent: its key was used for a new stream.
    StreamReset,
    /// The change feed ended: the store was closed.
    FeedEnded,
    /// The client sent something that is not a JSON `Ack`.
    InvalidInput(String),
    /// A socket read or write failed.
    Socket(String),
    /// A store read or send failed, or a send was refused.
    Store(String),
    /// A socket write, a read of the session or the send of an ack did not end within [`IO_TIMEOUT`].
    TimedOut(&'static str),
}

/// The store operations a connection uses. [`ActorSystem`] implements them with its public reads, sends and change
/// feed; a test wraps it to fail or stall them.
pub trait SessionStore: Send + Sync {
    /// The session's state, or `None` when it does not exist.
    fn read_session(&self, session: &Session) -> impl Future<Output = Result<Option<SessionState>, String>> + Send;

    /// Sends the client's ack to the session. `Ok` only when the message was accepted.
    fn send_ack(
        &self,
        session: &Session,
        stream: String,
        through: i64,
    ) -> impl Future<Output = Result<(), String>> + Send;

    /// The change hints under the session's address, a literal prefix: they may name other addresses.
    fn follow(&self, session: &Session) -> BoxStream<'static, ActorChange>;
}

impl SessionStore for ActorSystem {
    async fn read_session(&self, session: &Session) -> Result<Option<SessionState>, String> {
        let address = session.address().map_err(|e| e.to_string())?;
        match self.get_actor(&address).await.map_err(|e| e.to_string())? {
            Some(snapshot) => serde_json::from_value(snapshot.state).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }

    async fn send_ack(&self, session: &Session, stream: String, through: i64) -> Result<(), String> {
        match self.send(session, SessionMsg::Ack { stream, through }).await.map_err(|e| e.to_string())? {
            SendResult::Accepted(_) => Ok(()),
            refused => Err(format!("The ack was refused: {refused:?}")),
        }
    }

    fn follow(&self, session: &Session) -> BoxStream<'static, ActorChange> {
        match session.address() {
            Ok(address) => self.changes(address.as_str()).boxed(),
            Err(_) => futures::stream::empty().boxed(),
        }
    }
}

/// Serves `session` on one socket until the connection ends, and says why. `outbound` takes the JSON text of
/// [`ServerMessage`]s; `inbound` yields what the client sent and ends when the client closes. Both are borrowed, so
/// the caller can close the socket afterwards; neither is used once this returns. `shutdown` ends the connection when
/// it completes.
pub async fn serve<S, Tx, Rx, TxError, RxError>(
    store: &S,
    session: &Session,
    outbound: &mut Tx,
    inbound: &mut Rx,
    shutdown: impl Future<Output = ()>,
) -> Closed
where
    S: SessionStore,
    Tx: Sink<String, Error = TxError> + Unpin,
    Rx: Stream<Item = Result<Inbound, RxError>> + Unpin,
    TxError: Display,
    RxError: Display,
{
    if session.address().is_err() {
        return Closed::SessionMissing;
    }
    // Subscribe before the first read, so that no commit after that read goes unnoticed.
    let hints = store.follow(session);
    tokio::select! {
        result = write(store, session, hints, outbound) => match result {
            Err(closed) => closed,
        },
        closed = read(store, session, inbound) => closed,
        () = shutdown => Closed::Shutdown,
    }
}

/// Writes `Hello` and then the frames of each read; returns only with the reason the connection ends.
async fn write<S: SessionStore, Tx: Sink<String, Error = E> + Unpin, E: Display>(
    store: &S,
    session: &Session,
    mut hints: BoxStream<'static, ActorChange>,
    outbound: &mut Tx,
) -> Result<Infallible, Closed> {
    let address = session.address().map_err(|_| Closed::SessionMissing)?;
    // The first tick is at once: the first read waits for no hint.
    let mut poll = tokio::time::interval(POLL_INTERVAL);
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // The highest sequence this socket was sent; `None` until `Hello`.
    let mut sent: Option<u64> = None;
    loop {
        tokio::select! {
            hint = hints.next() => match hint {
                None => return Err(Closed::FeedEnded),
                Some(ActorChange::Changed { address: changed, .. }) if changed != address => continue,
                Some(_) => {}
            },
            _ = poll.tick() => {}
        }
        let state = match timeout(IO_TIMEOUT, store.read_session(session)).await {
            Err(_) => return Err(Closed::TimedOut("read")),
            Ok(Err(e)) => return Err(Closed::Store(e)),
            Ok(Ok(None)) => return Err(Closed::SessionMissing),
            Ok(Ok(Some(state))) => state,
        };
        let mut through = match sent {
            None => {
                send(outbound, &ServerMessage::Hello { stream: session.stream.clone(), acked: state.acked }).await?;
                state.acked
            }
            Some(sent) if state.assigned < sent => return Err(Closed::StreamReset),
            Some(sent) => sent.max(state.acked),
        };
        let from = through;
        for frame in state.frames.iter().filter(|f| f.seq > from) {
            let message =
                ServerMessage::Frame { stream: session.stream.clone(), seq: frame.seq, body: frame.body.clone() };
            send(outbound, &message).await?;
            through = frame.seq;
        }
        sent = Some(through);
    }
}

/// One socket write, bounded by [`IO_TIMEOUT`]; `send` flushes, so each message is written at once.
async fn send<Tx: Sink<String, Error = E> + Unpin, E: Display>(
    outbound: &mut Tx,
    message: &ServerMessage,
) -> Result<(), Closed> {
    let text = serde_json::to_string(message).expect("a server message encodes");
    match timeout(IO_TIMEOUT, outbound.send(text)).await {
        Err(_) => Err(Closed::TimedOut("write")),
        Ok(Err(e)) => Err(Closed::Socket(e.to_string())),
        Ok(Ok(())) => Ok(()),
    }
}

/// Forwards each client `Ack` to the session; returns with the reason the connection ends.
async fn read<S: SessionStore, Rx: Stream<Item = Result<Inbound, E>> + Unpin, E: Display>(
    store: &S,
    session: &Session,
    inbound: &mut Rx,
) -> Closed {
    loop {
        let text = match inbound.next().await {
            None => return Closed::ClientClosed,
            Some(Err(e)) => return Closed::Socket(e.to_string()),
            Some(Ok(Inbound::Binary)) => return Closed::InvalidInput("a binary message".into()),
            Some(Ok(Inbound::Text(text))) => text,
        };
        let ClientMessage::Ack { stream, through } = match serde_json::from_str(&text) {
            Ok(message) => message,
            Err(e) => return Closed::InvalidInput(e.to_string()),
        };
        // An uncertain outcome closes: the session keeps its acknowledgement, and a reconnect replays from it.
        match timeout(IO_TIMEOUT, store.send_ack(session, stream, through)).await {
            Err(_) => return Closed::TimedOut("ack"),
            Ok(Err(e)) => return Closed::Store(e),
            Ok(Ok(())) => {}
        }
    }
}
