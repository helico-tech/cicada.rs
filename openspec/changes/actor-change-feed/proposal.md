# Proposal

## Why

A live host page currently polls every actor or uses a TurnObserver that sees only its local node. Neither gives a
host a cross-node hint that its displayed actors changed. The preceding state listing removes N+1 reads; a feed can
avoid polling every row on every refresh. The code-factory showcase is the motivating host.

## What Changes

- Add ActorSystem::changes(prefix), a best-effort subscription yielding changed actor addresses and Resync hints.
  The subscriber reads one actor after Changed and relists the prefix after Resync, including at initial connection.
- Report creations, state writes, pause/resume and removals, including removed descendants. Events carry no state,
  version or change kind: a fresh read is authoritative, and versions reset when an address is recreated.
- PostgreSQL subscribers observe upgraded writers across nodes. Memory and SQLite cover writes through the same
  Store and its clones; separately opened SQLite handles, including other processes, are outside this first version.
- Bound subscriber buffering, signal detected gaps with Resync, and define startup/reconnect/close behaviour without
  a durable log or a replay cursor. The design must state precisely which losses are detected and how hosts recover.
- Keep dispatch wake-up filtering and core transaction guarantees; accept ordinary PostgreSQL notification failures.
  ADR 0034 records the feed's best-effort contract and inter-node notification format; no SQL schema change is planned.

The owner delegated the split to an Opus 5.5 adviser: this core feed first, then actor-change-sse for HTTP transport.
No TurnEnd extension is needed: an observer sees neither other nodes nor all operator-created snapshot changes.

## Settled boundaries

- ActorChange carries Changed { address } or Resync; the first/recovery resync waits for source readiness. A store
  owns the feed, so an unstarted system can subscribe and worker stop does not end it; Store::close does.
- One bounded transaction collector coalesces large batches. PostgreSQL uses one @-prefixed feed payload on the
  existing channel, atomically with its write; queue exhaustion/timeouts retain ordinary store-failure semantics.
- Memory/SQLite publish locally after commit; SQLite cancellation settles the writer before resync where the runtime
  remains available. Runtime shutdown and out-of-scope writers require a fresh subscription or explicit refresh.

## Capabilities

### New Capabilities

- `change-feed`: a host subscribes to hints that actors under a literal address prefix may have changed.

### Modified Capabilities

- `store-backends`: change streams end normally on close, an explicit exception to unavailable read/write calls.
  The internal listener owns reconnect readiness; ordinary wake-up filtering and TurnObserver remain unchanged.

## Out of scope

HTTP/SSE (the next change), state or version in events, durable history/replay, exactly-once delivery, a snapshot
across pages, state-field filters, changes to claiming/dispatch, SQL schema changes, SQLite writes through other
Store handles/processes, CLI/TUI integration and a TurnEnd extension. Hosts needing coverage outside the feed's
stated scope must keep their own refresh/recovery path.

## Order

After `list-actors-with-state`, before `actor-change-sse`.

## Impact

`crates/cicada`: ActorSystem and public change types, private store forwarding, memory/SQLite publication,
PostgreSQL listener/publication, conformance and runtime tests. ADR 0034, architecture, core guide, operations,
changelog/version. No HTTP, client, actor contract or generated OpenAPI change in this piece.
