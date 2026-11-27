# Docs handoff for client-change-stream

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-client` exports `ActorClient::changes`, `ChangeStream`, `Change` and `ChangeStreamError` (ADR 0077). The
client opens a Server-Sent Events subscription for a literal actor address prefix and yields changed addresses and
resync hints. The method's rustdoc example compiles in `cicada-client`. `crates/cicada-client/tests/changes.rs`
covers authentication and context headers, real store hints, an empty end at store close, open and read errors,
credential handling and the per-request timeout override. `crates/cicada-client/src/changes/tests.rs` covers split
frames, malformed events, idle and lifetime deadlines, and keep-alive comments.

## 1. Subscribe with the Rust client

**Where.** `docs/guide/http.md`, `The client`. Remove the sentence saying `ActorClient` has no SSE method from
`Follow changes over HTTP`.

**Text.** `ActorClient::changes(prefix)` opens a stream of best-effort actor change hints. Use the literal address
prefix from the listing calls, or an empty prefix to follow every actor. The method uses the same configured HTTP
client, including authentication headers and TLS settings. The example below is compiled in the rustdoc for
`ActorClient::changes`.

```rust,no_run
use cicada_client::{ActorClient, Change};
use futures::StreamExt;

async fn follow(client: &ActorClient) -> Result<(), Box<dyn std::error::Error>> {
    let mut changes = client.changes("counter/").await?;
    while let Some(change) = changes.next().await {
        match change? {
            Change::Changed(address) => { /* Reread this address and update the view. */ }
            Change::Resync => { /* Relist the prefix, across all pages. */ }
            _ => {}
        }
    }
    Ok(())
}
```

**Caveats.** The `changes` call returns `ClientError::Api` for a non-200 status and `ClientError::Body` if headers
take more than 10 seconds or the response is a `200` with another or missing media type. The stream ends cleanly
when the store closes. It yields a final `ChangeStreamError` on a failed read, invalid changed data, a line longer
than 64 KiB, 45 seconds without receiving body bytes, or a 24-hour lifetime. Keep-alive comments reset the idle
deadline. The request overrides the configured total timeout with 24 hours and 1 minute. A reconnect is a fresh
subscription that starts with a resync. Open it yourself and reread the prefix. The same change hint coverage and
reconciliation rules apply as for a browser client.

## 2. Update the architecture limits

**Where.** `docs/architecture.md`, `Known limitations`, final paragraph describing change hints and `GET /changes`.

**Text.** Replace the final line, `PostgreSQL nodes or SQL by hand; cicada-client has no method for the stream of GET
/changes.`, with `PostgreSQL nodes or SQL by hand. The Rust client reads GET /changes with ActorClient::changes and
leaves reconnection to the caller.` Keep code formatting around the names in the guide.

**Caveats.** The client has the same best-effort coverage, no history and no replay as the HTTP change feed.

## Blocking

None.

## Status

The 12 focused client tests, the `ActorClient::changes` doctest, the 11 documentation tests and the 54 example
doctests passed. The HTTP guide and architecture edits are proposed for the docs session. ADR 0077 and the changelog
are in the implementation branch.
