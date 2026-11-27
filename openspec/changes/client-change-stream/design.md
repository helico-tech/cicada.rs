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

1. **ADR 0077 lifts ADR 0035's "no client streaming API".** ADR 0035's Consequences say `cicada-client` gets no
   streaming API for `/changes`. A short ADR 0077 partly supersedes that line: the client gains a reader, still without
   replay or reconnection, and `futures-core` (already in the tree) becomes a public dependency of `cicada-client` for
   its `Stream` type. ADR 0035's status line says it is partly superseded by 0077.
2. **Opening the stream: errors from the call.** `ActorClient::changes(&self, prefix: &str) ->
   ClientResult<ChangeStream>` sends its request through the `ActorClient`'s `reqwest::Client`, so its default headers
   (the token, the context's headers) and TLS settings apply unchanged. The call waits at most 10 s for the response
   headers: past that it returns `ClientError::Http` with a timeout, as other calls report a timeout. A status other
   than 200 returns `ClientError::Api` with the problem (or one made from the status), as every other call does, so a
   caller tells a 401 from a 404. A 200 whose content type is not `text/event-stream` (a proxy page, a wrong base URL)
   returns `ClientError::Body`.
3. **Reading it: errors in the stream.** The request sets its own timeout of 24 hours and 1 minute, which replaces the
   client's total timeout for this request only. The stream itself ends with `Lifetime` after 24 hours on its own Tokio
   timer, and the caller opens a new one. It ends with `Idle` when no byte at all, keep-alive comments included, has
   arrived for 45 s (three keep-alive periods), also on Tokio's clock. A failed read, including a client `read_timeout`
   that fires, is `Transport`. A body that ends cleanly ends the stream with `None` (a store close).
4. **A parser by the event-stream rules.** The body is read with `Response::chunk()` (reqwest's `stream` feature stays
   off). A UTF-8 BOM at the very start is dropped, also when it is split over reads; lines end with CRLF, LF or CR, also
   split over reads; a field's value drops one space after the colon; a line with no colon is a field name with an empty
   value; `data` lines of one event join with `\n`; a line starting with `:` is a comment; an empty line ends the event;
   `event` names it; unknown fields and events are skipped; an incomplete event when the body ends is dropped. A line
   longer than 64 KiB, or a `changed` whose data is not `{"address": <valid address>}`, is `Malformed` (the stream
   ends).
5. **The types.** `ChangeStream` is a boxed `Stream<Item = Result<Change, ChangeStreamError>> + Unpin + Send`; `Change`
   is `Changed(ActorAddress)` or `Resync`, `#[non_exhaustive]`; `ChangeStreamError` (`Idle`, `Lifetime`, `Transport`,
   `Malformed`) is `#[non_exhaustive]` and never holds a token or a URL password in its text (reqwest removes URL
   credentials from its errors; a test checks it). It never reconnects; the caller decides when to open the next stream.
   Additive: no break, `docs/openapi.json` unchanged.

## Tests

Against an in-process server (`cicada-axum`'s router over `TestActors`): `changed` and `resync` from real changes, the
token and a context header arriving at the server, a store close ending the stream with `None`, a 401 and a 404 as
`ClientError::Api` with their status, a 200 with another content type as `Body`, no token or URL password in any error's
text. With a small local server in real time: a client built with a total timeout of 1 s keeps a stream open for 3 s of
bytes every 200 ms. With crafted byte streams on virtual time: a BOM split over two reads, CRLF, LF and CR endings split
over reads, `event: changed` with its space, a line with no colon, multi-line `data`, a comment, an unknown event, an
incomplete final event, a line over 64 KiB, a malformed `changed`; `Idle` after 45 s with no byte and keep-alives
keeping it open; the 10 s handshake timeout; `Lifetime` at 24 hours.

## Docs

The handoff gives the HTTP guide's client section `ActorClient::changes` with the example above, and replaces the line
of `docs/architecture.md` (its limits paragraph) that says `cicada-client` has no method for `/changes`; the
changelog's 0.26.0 section names it under Added. ADR 0077 is written by the change, with its row in the index.
