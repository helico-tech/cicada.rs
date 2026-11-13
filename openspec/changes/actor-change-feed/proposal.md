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
- Keep the existing dispatch wake-ups and turn outcomes intact. ADR 0034 will record the feed's best-effort contract
  and any inter-node notification format; no SQL schema change is planned.

The owner delegated the split to an Opus 5.5 adviser: this core feed first, then actor-change-sse for HTTP transport.
No TurnEnd extension is needed: an observer sees neither other nodes nor all operator-created snapshot changes.

## To decide

- The exact source readiness and resync protocol, including races with subscription and shutdown.
- PostgreSQL publication failure boundaries: no payload/queue failure should be hidden behind a claim of reliability.
- The public stream type, buffer policy, and explicit limits of best-effort coverage and recovery.

## Capabilities

### New Capabilities

- `change-feed`: a host subscribes to hints that actors under a literal address prefix may have changed.

### Modified Capabilities

None planned; dispatch wake-ups and turn observers retain their behaviour.

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
