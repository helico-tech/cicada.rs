//! The bridge behind an axum websocket route: `GET /sessions/{stream}/socket`.
//!
//! The route authenticates nobody: knowing a stream key is not permission to read it. A host puts its own
//! authentication in front of [`router`] and checks that the caller is the session's logical client before the
//! upgrade. Inbound messages and frames are limited to [`MAX_INBOUND_BYTES`]; each outbound message is written at
//! once, with at most [`MAX_WRITE_BUFFER_BYTES`] buffered. The websocket library answers pings; a binary or malformed
//! message closes the connection. Each connection runs in the task axum gives the upgraded socket, and ends when
//! `shutdown` is cancelled.

use std::future::ready;
use std::pin::pin;
use std::sync::Arc;

use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use futures::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::Session;
use super::bridge::{Closed, IO_TIMEOUT, Inbound, SessionStore, serve};

/// The largest message or frame a client may send.
pub const MAX_INBOUND_BYTES: usize = 4 * 1024;

/// The most bytes a connection buffers for writing.
pub const MAX_WRITE_BUFFER_BYTES: usize = 16 * 1024;

struct Shared<S> {
    store: S,
    shutdown: CancellationToken,
}

/// The session route, served from `store`; cancelling `shutdown` ends every connection.
pub fn router<S: SessionStore + 'static>(store: S, shutdown: CancellationToken) -> Router {
    Router::new().route("/sessions/{stream}/socket", get(upgrade::<S>)).with_state(Arc::new(Shared { store, shutdown }))
}

async fn upgrade<S: SessionStore + 'static>(
    State(shared): State<Arc<Shared<S>>>,
    Path(stream): Path<String>,
    socket: WebSocketUpgrade,
) -> Response {
    socket
        .max_message_size(MAX_INBOUND_BYTES)
        .max_frame_size(MAX_INBOUND_BYTES)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_WRITE_BUFFER_BYTES)
        .on_upgrade(move |socket| connection(shared, Session { stream }, socket))
}

async fn connection<S: SessionStore>(shared: Arc<Shared<S>>, session: Session, socket: WebSocket) {
    let (mut sink, stream) = socket.split();
    let mut inbound = pin!(stream.filter_map(|message| {
        ready(match message {
            Ok(Message::Text(text)) => Some(Ok(Inbound::Text(text.to_string()))),
            Ok(Message::Binary(_)) => Some(Ok(Inbound::Binary)),
            // The library answers pings and ends the stream after a close.
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Close(_)) => None,
            Err(e) => Some(Err(e)),
        })
    }));
    let closed = {
        let mut outbound = (&mut sink).with(|text: String| ready(Ok::<_, axum::Error>(Message::Text(text.into()))));
        serve(&shared.store, &session, &mut outbound, &mut inbound, shared.shutdown.cancelled()).await
    };
    let code = match closed {
        Closed::ClientClosed => close_code::NORMAL,
        Closed::Shutdown => close_code::AWAY,
        Closed::InvalidInput(_) => close_code::UNSUPPORTED,
        Closed::SessionMissing | Closed::StreamReset => close_code::POLICY,
        _ => close_code::ERROR,
    };
    // A best-effort close frame; the socket is dropped either way.
    let _ = timeout(IO_TIMEOUT, sink.send(Message::Close(Some(CloseFrame { code, reason: "".into() })))).await;
}
