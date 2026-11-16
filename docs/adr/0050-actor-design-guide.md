# 0050. Actor design knowledge in a guide with a checked websocket example

Date: 2026-11-16

## Status

Accepted for `actor-design-guide` after design review and owner approval on 2026-11-16. Extends ADR 0041's placement
of reusable design knowledge in the core guide.
No Cicada runtime guarantee, library API, HTTP route, ready-made actor contract or SQL schema changes.

## Context

Consumers need to decide actor boundaries and protocols before writing handlers. The research is broader than an
API guide, and the owner chose three changes: a guide, an interactive design skill and that skill's review mode.
The guide needs a concrete example of durable actor state coordinating with a live handle held by a host.

## Decision

- Put the expanded knowledge in docs/guide/design.md. Explain a principle through a story, then its Cicada form;
  keep API detail in the layer guides. Show the nine steps' artifacts, patterns and mistakes with diagrams.
  Research claims are checked against current architecture and source, not promoted to universal guarantees.
- Use a websocket session as the checked example. One logical client owns one never-reused stream key; the host
  owns the socket and may run on a different node from the actor. No pin or live connection registry is needed.
- The application protocol has Hello, Frame and Ack. Persist up to 100 frames of at most 1024 UTF-8 bytes without
  NUL, positive sequences bounded to JavaScript's exact integer range, and a cumulative processed ack. Enqueue
  appends and replies atomically; refuse full buffers explicitly instead of dropping accepted frames. Unknown
  transport acceptance is distinct from the application reply. Missing reply destinations retain normal rollback.
  NUL is already refused by Cicada before submission reaches the actor; oversized, otherwise storable bodies get
  the example's InvalidBody reply. Keep that distinction in tests and guidance without changing core validation.
- Only a valid monotone Ack trims frames. Socket writes are outside effects; reconnect and refresh may replay.
  The client retains a contiguous cursor, deduplicates and re-acks. Its lost cursor cannot reconstruct already
  acknowledged history. Deletion ends the stream and requires a new key; no exactly-once client promise is added.
- Subscribe before reading and reconcile on hints, resync and a timer. Bound connection I/O and stop both halves
  on failure/shutdown. Preserve change-feed scope and the host's responsibility for client authorization.
- The docs owner prepares prose; the builder owns example code, tests, doctest wiring and routes. Landing waits
  for the owner's exact guide/navigation (including root README) and G2 test references. Interactive skill behavior
  and packaging follow.

## Consequences

Both future skill modes can route to one guide without copying knowledge. The websocket contract is an example's
application convention, not a new Cicada HTTP API or delivery guarantee. A bounded replay buffer does not bound
mailboxes or total sessions. The example deliberately excludes shared subscribers, automatic eviction and socket
fencing. Existing guide anchors and public behavior remain intact; a new guide and routes must land together.
