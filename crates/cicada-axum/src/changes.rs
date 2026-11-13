//! `GET /changes`: the actor change hints of [`cicada::change`] as Server-Sent Events (ADR 0035).
//!
//! `Changed` is the event `changed` with data `{"address":"<canonical>"}`, `Resync` the event `resync` with data `{}`.
//! No `id` or `retry` field: there is no replay, and every request is a new subscription that starts with a resync. A
//! comment `keep-alive` follows each 15 seconds without output; it says nothing about readiness. The body ends when
//! the stream of hints ends, which a closed store does.

use std::time::Duration;

use axum::http::{HeaderName, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use cicada::ActorChange;
use futures::{Stream, StreamExt};
use serde::Serialize;

/// The longest time without output before a keep-alive comment.
pub(crate) const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The data of a `changed` event.
#[derive(Serialize)]
struct Changed<'a> {
    address: &'a str,
}

/// The SSE frame of one hint. A variant this crate does not know yet asks for a full reread, like a resync.
fn event(change: ActorChange) -> Result<Event, axum::Error> {
    match change {
        ActorChange::Changed { address, .. } => {
            Event::default().event("changed").json_data(Changed { address: address.as_str() })
        }
        _ => Ok(Event::default().event("resync").data("{}")),
    }
}

/// The `200 text/event-stream` response of `changes`: one frame per hint as the body is read, with no queue of its
/// own, keep-alive comments while the stream is pending, and the proxy hint `X-Accel-Buffering: no`.
pub(crate) fn event_stream(changes: impl Stream<Item = ActorChange> + Send + 'static) -> Response {
    let sse = Sse::new(changes.map(event)).keep_alive(KeepAlive::new().interval(KEEP_ALIVE).text("keep-alive"));
    ([(HeaderName::from_static("x-accel-buffering"), HeaderValue::from_static("no"))], sse).into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::header;
    use futures::stream::BoxStream;
    use tokio::time::Instant;

    use super::*;

    async fn frame(body: &mut BoxStream<'static, Result<axum::body::Bytes, axum::Error>>) -> Option<String> {
        body.next().await.map(|chunk| String::from_utf8(chunk.unwrap().to_vec()).unwrap())
    }

    fn body_of(response: Response) -> BoxStream<'static, Result<axum::body::Bytes, axum::Error>> {
        Body::new(response.into_body()).into_data_stream().boxed()
    }

    /// Scenario: Source is not yet ready.
    #[tokio::test(start_paused = true)]
    async fn a_pending_source_gets_keep_alive_comments_every_15_seconds_and_no_event() {
        let response = event_stream(futures::stream::pending());
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/event-stream");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(response.headers()["x-accel-buffering"], "no");
        let mut body = body_of(response);
        let started = Instant::now();
        for n in 1..=3 {
            assert_eq!(frame(&mut body).await.as_deref(), Some(": keep-alive\n\n"));
            assert_eq!(started.elapsed(), KEEP_ALIVE * n);
        }
    }

    /// Scenarios: First application event; Idle after an event.
    #[tokio::test(start_paused = true)]
    async fn an_event_resets_the_keep_alive_and_an_ended_source_ends_the_body() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut body = body_of(event_stream(tokio_stream_of(rx)));
        tokio::time::sleep(Duration::from_secs(10)).await;
        tx.send(ActorChange::Resync).unwrap();
        let sent = Instant::now();
        assert_eq!(frame(&mut body).await.as_deref(), Some("event: resync\ndata: {}\n\n"));
        assert_eq!(frame(&mut body).await.as_deref(), Some(": keep-alive\n\n"));
        assert_eq!(sent.elapsed(), KEEP_ALIVE, "15 seconds after the event, not after the start");
        drop(tx);
        assert_eq!(frame(&mut body).await, None, "no comments after the end");
    }

    fn tokio_stream_of(
        mut rx: tokio::sync::mpsc::UnboundedReceiver<ActorChange>,
    ) -> impl Stream<Item = ActorChange> + Send + 'static {
        futures::stream::poll_fn(move |cx| rx.poll_recv(cx))
    }
}
