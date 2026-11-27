# Proposal

## Why

`GET /changes?prefix=` streams change hints as Server-Sent Events (ADR 0035), but `cicada-client` has no reader for
them. The inspector's live updates need one, and so does any Rust client that wants to follow changes. Split from
`inspector-live` by its design review (2026-11-27): the transport is a change of its own.

## What Changes

- `ActorClient::changes(prefix)`: a stream of changed addresses and resyncs, through the client's own HTTP client, so
  its authentication headers and TLS settings apply, with a bounded handshake, an idle limit measured from the last
  byte (keep-alives included), and typed errors. It never reconnects by itself.

## Order

After `inspector-nodes`, before `inspector-soft-refresh`.

## Impact

Crates: cicada-client.

`crates/cicada-client` (the stream and its parser). Tests against an in-process server and with crafted byte streams.
ADR 0077, partly superseding ADR 0035's "no client streaming API". Docs handoff: the HTTP guide and the
limits paragraph of `docs/architecture.md`. Additive.
