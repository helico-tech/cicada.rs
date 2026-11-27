# 0077. A client for the change stream

Date: 2026-11-27

## Status

Accepted for `client-change-stream`. Partly supersedes [ADR 0035](0035-actor-change-sse.md)'s decision to leave the
Rust client without a streaming API for `GET /changes`.

## Context

The inspector and other Rust consumers need the same best-effort change hints as browser consumers. The HTTP client
already carries authentication and TLS settings, and may set a total timeout suited to ordinary calls.

## Decision

- `ActorClient::changes(prefix)` opens `GET /changes` through its configured HTTP client. It waits up to 10 s for
  headers and checks the status and media type before returning a stream. The request overrides the client's total
  timeout with 24 hours and 1 minute.
- The stream reads ahead with a bounded buffer and yields changed addresses and resync hints. It ends cleanly when
  the server closes the body. It reports a read or framing error, 45 s without a byte, or an elapsed lifetime of
  24 hours as a final error. Keep-alive comments reset the idle deadline even between calls to `next`. Both a line
  and an event's joined data are limited to 64 KiB. `futures-core` is a public dependency for the `Stream` type.

## Consequences

- Consumers choose when to reconnect and read current data after hints. Each connection is a fresh subscription with
  the existing resync behavior and no replay. The server contract and OpenAPI document stay the same.
