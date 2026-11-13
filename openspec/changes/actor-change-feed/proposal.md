# Proposal

## Why

Nothing tells a host that an actor's state changed. A live page has two ways today, and neither is enough:

- Poll the API. Simple, but always behind by the interval, and it reads everything on every round.
- Register a `TurnObserver`. It runs after a turn's commit, but in-process and per node: with several nodes it misses
  the turns of the others. It also sees neither the new state nor its version, only the outcome and the sends.

The store already knows: on PostgreSQL every commit that makes a message claimable sends a `NOTIFY` on
`cicada_<schema>`. That listener is internal (`pub(crate)`), its payload is a type name, and it serves only to wake
the dispatch loop.

Found in cicada-code-factory: its showcase page polls once a second. Push through an observer would work on its one
node, and break the day it runs a second.

## What Changes

- A public subscription to committed changes of actors, across nodes: each event names the actor and its new state
  version (not the state; the subscriber reads what it needs). In-process as a stream on `ActorSystem`
  (`changes(prefix)`), over HTTP as Server-Sent Events (`GET /changes?prefix=`), behind the host's layers like the
  rest of the API.
- Best effort, with a way back: after a lost connection or a dropped event the stream says "resync", as the internal
  listener does with its empty payload, and the subscriber reads again.
- This is more than one small change. The design loop splits it, for instance: (1) `TurnEnd` tells the observer
  whether the turn wrote state, and its new version (per node, no store change); (2) the cross-node stream on
  `ActorSystem`; (3) the SSE route.

## To decide

- What an event carries: address and version (the proposal), or the state too (bigger payloads, `NOTIFY` is limited
  to 8000 bytes).
- How PostgreSQL carries it: the existing channel with a richer payload, or a channel of its own; batching of many
  commits; what the dispatch loop's wake-ups keep.
- SQLite: in-process only, or seeing the commits of other processes on the same file (WAL; a poll of
  `data_version`).
- Whether creates and deletes are events too, and whether a change of a mailbox (a paused actor) is.
- A slow subscriber: a bounded buffer that ends in "resync", or a dropped stream.
- Whether it is a new guarantee in `docs/architecture.md` (then an ADR), and how it relates to watches (ADR 0018,
  ADR 0023), which notify actors, not hosts.

## Capabilities

### New Capabilities

- `change-feed`: a host can subscribe to committed changes of actors under a prefix, across nodes.

### Modified Capabilities

- `turn-observer` (if the first piece is the observer's state version), `store-backends`, `management-api`.

## Out of scope

The state in the event; a durable log of past changes or replay from a version; exactly-once delivery; filtering on
fields of the state; any change to how turns are claimed or dispatched.

## Order

After `list-actors-with-state`, the last of the line.

## Impact

`crates/cicada` (the PostgreSQL listener and the SQLite backend, `ActorSystem`, perhaps `TurnEnd`), the conformance
suite, `cicada-axum` (an SSE route) and `docs/openapi.json`, perhaps `cicada-client`, `docs/architecture.md` and an
ADR if it is a guarantee, the core and HTTP guides, the changelog. Once it is released, cicada-code-factory's page
can be pushed to instead of polling (ADR 0007 of that repository).
