use std::pin::Pin;
use std::time::Duration;

use cicada::ActorAddress;
use futures_core::Stream;
use futures_util::stream::{StreamExt, try_unfold, unfold};
use reqwest::{StatusCode, header::CONTENT_TYPE};
use serde::Deserialize;
use tokio::time::{Instant, sleep_until, timeout_at};

use crate::{ActorClient, Answer, ClientError, ClientResult};

const HEADER_WAIT: Duration = Duration::from_secs(10);
const IDLE_LIMIT: Duration = Duration::from_secs(45);
const LIFETIME: Duration = Duration::from_hours(24);
const REQUEST_LIMIT: Duration = LIFETIME.saturating_add(Duration::from_mins(1));
const MAX_LINE: usize = 64 * 1024;
const BOM: &[u8] = b"\xef\xbb\xbf";

/// A best-effort change hint from [`ActorClient::changes`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// Read this actor's current state again. It may have been removed.
    Changed(ActorAddress),
    /// Read the whole subscribed prefix again, including on a fresh subscription.
    Resync,
}

/// The final error of an open [`ChangeStream`]. The stream ends after yielding it once.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChangeStreamError {
    /// No body bytes arrived for 45 seconds.
    #[error("no change stream bytes within 45 s")]
    Idle,
    /// The stream has been open for 24 hours.
    #[error("change stream lifetime of 24 hours expired")]
    Lifetime,
    /// Reading the response body failed, including a timeout on the underlying HTTP read.
    #[error("could not read the change stream body")]
    Transport,
    /// A line exceeded 64 KiB or a `changed` event had invalid data.
    #[error("malformed change stream: {0}")]
    Malformed(&'static str),
}

/// A boxed, `Unpin` and `Send` stream of change hints. A clean end of the HTTP body yields `None`.
pub type ChangeStream = Pin<Box<dyn Stream<Item = Result<Change, ChangeStreamError>> + Send>>;

impl ActorClient {
    /// Opens `GET /changes?prefix=` for a literal address prefix (empty for all actors).
    ///
    /// The client's HTTP settings apply, with a total timeout of 24 hours and 1 minute for this request only.
    /// A non-200 response returns [`ClientError::Api`]. No headers within 10 seconds or a `200` without
    /// `text/event-stream` returns [`ClientError::Body`]. Once open, any byte, including a keep-alive, resets a
    /// 45-second idle deadline. A stream lasts at most 24 hours. Open a new stream yourself after an error or EOF.
    /// Hints carry no state and have no replay.
    ///
    /// ```no_run
    /// use cicada_client::{ActorClient, Change};
    /// use futures::StreamExt;
    ///
    /// # async fn follow(client: &ActorClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut changes = client.changes("counter/").await?;
    /// while let Some(change) = changes.next().await {
    ///     match change? {
    ///         Change::Changed(address) => { /* Reread this address and update the view. */ }
    ///         Change::Resync => { /* Relist the prefix, across all pages. */ }
    ///         _ => {}
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn changes(&self, prefix: &str) -> ClientResult<ChangeStream> {
        let url = self.url("/changes", &[("prefix", prefix.to_owned())])?;
        let request = self.http.get(url).timeout(REQUEST_LIMIT);
        let open_by = Instant::now() + HEADER_WAIT;
        let response = timeout_at(open_by, request.send())
            .await
            .map_err(|_| ClientError::Body("no response headers within 10 s".to_owned()))?
            .map_err(|error| ClientError::Http(error.without_url()))?;
        let status = response.status();
        if status != StatusCode::OK {
            let problem = timeout_at(open_by, response.json())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_else(|| Answer::status_problem(status));
            return Err(ClientError::Api(problem));
        }
        let event_stream =
            response.headers().get(CONTENT_TYPE).and_then(|header| header.to_str().ok()).is_some_and(|value| {
                value.split(';').next().unwrap_or_default().trim().eq_ignore_ascii_case("text/event-stream")
            });
        if !event_stream {
            return Err(ClientError::Body("expected a text/event-stream response".to_owned()));
        }
        Ok(read_stream(try_unfold(response, |mut response| async move {
            let chunk = response.chunk().await.map_err(|_| ChangeStreamError::Transport)?;
            Ok(chunk.map(|bytes| (bytes, response)))
        })))
    }
}

fn read_stream<B>(chunks: impl Stream<Item = Result<B, ChangeStreamError>> + Send + 'static) -> ChangeStream
where
    B: AsRef<[u8]> + Send + 'static,
{
    let opened = Instant::now();
    let reader = Reader {
        chunks: Box::pin(chunks),
        chunk: None,
        offset: 0,
        idle_at: opened + IDLE_LIMIT,
        expires_at: opened + LIFETIME,
        parser: Parser::default(),
    };
    Box::pin(
        unfold(Some(reader), |reader| async {
            let mut reader = reader?;
            match reader.next_change().await {
                Ok(Some(change)) => Some((Ok(change), Some(reader))),
                Ok(None) => None,
                Err(error) => Some((Err(error), None)),
            }
        })
        .fuse(),
    )
}

struct Reader<S, B> {
    chunks: Pin<Box<S>>,
    chunk: Option<B>,
    offset: usize,
    idle_at: Instant,
    expires_at: Instant,
    parser: Parser,
}

impl<S, B> Reader<S, B>
where
    S: Stream<Item = Result<B, ChangeStreamError>>,
    B: AsRef<[u8]>,
{
    async fn next_change(&mut self) -> Result<Option<Change>, ChangeStreamError> {
        loop {
            if Instant::now() >= self.expires_at {
                return Err(ChangeStreamError::Lifetime);
            }
            if let Some(chunk) = &self.chunk {
                let bytes = chunk.as_ref();
                let mut rest = &bytes[self.offset..];
                let change = self.parser.consume(&mut rest)?;
                self.offset = bytes.len() - rest.len();
                if change.is_some() {
                    return Ok(change);
                }
            }
            let (deadline, elapsed) = if self.idle_at < self.expires_at {
                (self.idle_at, ChangeStreamError::Idle)
            } else {
                (self.expires_at, ChangeStreamError::Lifetime)
            };
            let chunk = tokio::select! {
                biased;
                _ = sleep_until(deadline) => return Err(elapsed),
                chunk = self.chunks.next() => chunk.transpose()?,
            };
            let Some(chunk) = chunk else { return Ok(None) };
            if !chunk.as_ref().is_empty() {
                self.idle_at = Instant::now() + IDLE_LIMIT;
            }
            self.chunk = Some(chunk);
            self.offset = 0;
        }
    }
}

#[derive(Default)]
struct Parser {
    line: Vec<u8>,
    seen_line: bool,
    after_cr: bool,
    event: Event,
    data: Vec<u8>,
}

#[derive(Default)]
enum Event {
    #[default]
    Unknown,
    Changed,
    Resync,
}

#[derive(Deserialize)]
struct Changed {
    address: ActorAddress,
}

impl Parser {
    /// Consumes through the first complete hint, retaining any partial line or event between reads.
    fn consume(&mut self, bytes: &mut &[u8]) -> Result<Option<Change>, ChangeStreamError> {
        while !bytes.is_empty() {
            if std::mem::take(&mut self.after_cr) && bytes[0] == b'\n' {
                *bytes = &bytes[1..];
                continue;
            }
            let end = bytes.iter().position(|&byte| matches!(byte, b'\r' | b'\n')).unwrap_or(bytes.len());
            let (part, rest) = bytes.split_at(end);
            let bom_allowance = if self.seen_line { 0 } else { BOM.len() };
            if self.line.len() + part.len() > MAX_LINE + bom_allowance {
                return Err(ChangeStreamError::Malformed("line exceeds 64 KiB"));
            }
            self.line.extend_from_slice(part);
            let line = if self.seen_line { &self.line } else { self.line.strip_prefix(BOM).unwrap_or(&self.line) };
            if line.len() > MAX_LINE {
                return Err(ChangeStreamError::Malformed("line exceeds 64 KiB"));
            }
            *bytes = rest;
            let Some((&ending, rest)) = bytes.split_first() else { break };
            *bytes = rest;
            self.after_cr = ending == b'\r';
            self.seen_line = true;

            let change = if line.is_empty() {
                self.dispatch()?
            } else {
                let mut fields = line.splitn(2, |&byte| byte == b':');
                let field = fields.next().unwrap_or_default();
                let value = fields.next().unwrap_or_default();
                let value = value.strip_prefix(b" ").unwrap_or(value);
                match field {
                    b"event" => {
                        self.event = match value {
                            b"changed" => Event::Changed,
                            b"resync" => Event::Resync,
                            _ => Event::Unknown,
                        };
                    }
                    b"data" => {
                        self.data.extend_from_slice(value);
                        self.data.push(b'\n');
                    }
                    _ => (),
                }
                None
            };
            self.line.clear();
            if change.is_some() {
                return Ok(change);
            }
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Result<Option<Change>, ChangeStreamError> {
        let event = std::mem::take(&mut self.event);
        let change = match (event, self.data.strip_suffix(b"\n")) {
            (Event::Changed, Some(data)) => {
                let data = String::from_utf8_lossy(data);
                let changed: Changed = serde_json::from_str(&data)
                    .map_err(|_| ChangeStreamError::Malformed("changed event has invalid address data"))?;
                Some(Change::Changed(changed.address))
            }
            (Event::Resync, Some(_)) => Some(Change::Resync),
            _ => None,
        };
        self.data.clear();
        Ok(change)
    }
}

#[cfg(test)]
mod tests;
