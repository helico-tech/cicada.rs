# Design

`GET /changes?prefix=` (ADR 0035) sends `changed` events with `{"address": …}` and `resync` events with `{}`, keep-alive
comments every 15 s, no ids and no replay, and ends without an event when the store closes. `cicada-client` has no
reader for it. A client made by `cicada_app::context_client` has a total request timeout (the operator commands' 30 s),
which would cut a healthy stream off (design review of `inspector-live`).

```rust
let mut changes = client.changes("counter/").await?;
while let Some(change) = changes.next().await {
    match change? { Change::Changed(address) => reload(address), Change::Resync => reload_all() }
}
```

## Decisions

1. **The client's own HTTP client, with a per-request timeout.** `changes` sends its request through the
   `ActorClient`'s `reqwest::Client`, so its default headers (the token, the context's headers) and its TLS settings
   apply unchanged. The request sets its own timeout of 24 hours, which replaces the client's total timeout for this
   request only; the stream then ends with `ChangeStreamError::Lifetime`, and the caller opens a new one. The handshake
   (until the response headers) is bounded by 10 s (`Handshake`); a status other than 200 is `Status` with the
   problem's text, as the other calls report it.
2. **An idle limit from the last byte.** The stream ends with `Idle` when no byte at all, keep-alive comments included,
   has arrived for 45 s (three keep-alive periods). Measured on Tokio's clock, so tests run on virtual time.
3. **A parser by the event-stream rules.** A UTF-8 BOM at the very start is dropped, also when it is split over
   reads; lines end with CRLF, LF or CR, also split over reads; `data` lines of one event join with `\n`; a line
   starting with `:` is a comment; an empty line ends the event; `event` names it; unknown fields and events are
   skipped; an incomplete event when the body ends is dropped. `changed` with data that is not `{"address": <valid
   address>}` is `Malformed` (the stream ends). A body that ends cleanly ends the stream with `None` (a store close).
4. **The API.** `ActorClient::changes(&self, prefix: &str) -> ClientResult<ChangeStream>`; `ChangeStream` is an
   `impl Stream<Item = Result<Change, ChangeStreamError>> + Unpin + Send`; `Change` is `Changed(ActorAddress)` or
   `Resync`, `#[non_exhaustive]`; `ChangeStreamError` (`Handshake`, `Status`, `Idle`, `Lifetime`, `Transport`,
   `Malformed`) is `#[non_exhaustive]` and never holds a token or a URL password in its text. It never reconnects;
   the caller decides when to open the next stream. Additive: no break, `docs/openapi.json` unchanged.

## Tests

Against an in-process server (`cicada-axum`'s router over `TestActors`): `changed` and `resync` from real changes, the
token and a context header arriving at the server, a store close ending the stream with `None`, a 404 or 401 as
`Status`. With crafted byte streams on virtual time: a BOM split over two reads, CRLF, LF and CR endings split over
reads, multi-line `data`, a comment, an unknown event, an incomplete final event, a malformed `changed`; `Idle` after
45 s with no byte, and keep-alives keeping it open; `Handshake` after 10 s without headers; `Lifetime` at 24 hours.

## Docs

The handoff gives the HTTP guide's client section `ActorClient::changes` with the example above; the changelog's 0.26.0
section names it under Added.
