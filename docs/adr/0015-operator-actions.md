# 0015. Operator actions

Date: 2026-11-09

## Status

Accepted

## Context

Operators want to see which nodes exist and what they run, stop a turn that runs away, hold or clear a mailbox, run a
delayed message now, and retry a failed message, one at a time or after an incident in bulk. ADR 0014 left a gap: a
create inside a commit took no lock on the type, so `DELETE /types/{name}` could leave an actor of a removed type. ADR
0008 allows changes to the wire format and the schema while there is no production database. The changes are
`node-view`, `turn-management`, `mailbox-management`, `message-actions` and `bulk-message-actions` in `openspec/`.

## Decision

- **Type registry.** The store itself refuses, atomically, to create an actor whose type is not registered:
  `Store::create` answers `CreateResult::NoSuchType`, and a commit whose creates name an unregistered type answers
  `CommitResult::Rejected` (the turn ends `failed`, as for a send to an absent instance). PostgreSQL takes `FOR SHARE`
  on the type's row in both paths and `remove_type` keeps `FOR UPDATE`. The look-up-after-insert of ADR 0014 is gone.
- **The node table is display only.** One row per node, send-only nodes too, written by a 10 s heartbeat with store
  time; a node removes its own row on stop, any heartbeat removes rows older than a day. Status is derived from the
  age of the last heartbeat in the store's clock (`alive` under 30 s, `suspect` under 2 min, `dead`). `GET /nodes` and
  `GET /nodes/{node}`, the latter with the node's running turns. Leases stay the only authority over who runs what, so
  a wrong status can never cause overlapping turns.
- **Running turns.** `GET /turns` and `GET /actors/{type}/{key}/turn` serve instances with a lease that is valid at
  store now. `POST /actors/{type}/{key}/turn/cancel` records the epoch to cancel (`actors.cancel_epoch`, only while a
  valid lease is held); the keeper reads it through `RenewResult::CancelRequested` at its next renewal, so a request
  takes up to one heartbeat. The owning node aborts the handler (it stops at its next `.await`; blocking code runs
  until it returns), waits for it to stop, and commits the new final outcome `TurnOutcome::Cancelled`: no effects, no
  retry, fenced by its epoch, `on_outcome` sees it and `on_handler_error` is not called. Which of the handler's end and
  a request came first is decided once by an atomic flag; a request that arrives after the handler ended is ignored (G6)
  and the turn commits normally. Storing the epoch means a stale request can never hit a later turn.
- **No forced lease release, on any node.** It was asked for and decided against: a dead node's lease expires by
  itself within the TTL, and on a live node it would break G1 (a second node could run the turn while the first still
  does) and G6 (the handler re-runs after it returned). The cooperative cancel above is the only way to stop a running
  turn.
- **Pausing changes S8.** `actors.paused`: `claim_due` and `next_due_at` skip paused actors, messages still queue up, a
  running turn finishes. S8 now says that a paused instance is not returned until it is resumed; the actor is not lost,
  only held. `POST .../mailbox/pause`, `/resume` (which wakes the dispatch loop and, on PostgreSQL, sends the `NOTIFY`).
  `actors_due` now omits paused actors from its partial predicate, so claims keep using the index without a table scan.
- **Purge.** `POST .../mailbox/purge` gives outcome `purged` to every pending message except one being handled under a
  valid lease; the actor stays. Purging a message whose lease expired voids that lease.
- **Retry is a new message.** `POST /messages/{id}/retry` is for a message that ended `failed` or `cancelled`: a new
  message with a new id, same target, type, payload, correlation id, headers, retry and retention, `causedBy` the
  original, sent through `Store::enqueue` with no sender. It is not idempotent: every call sends another message.
  Pending, running, done and purged messages answer `409 message-not-retryable`.
- **Deliver now.** `POST /messages/{id}/deliver-now` makes a pending, not running message due at store now with a new
  `seq`.
- **Bulk.** `POST /messages/bulk-cancel` and `/bulk-retry` take `ids` or a filter (`prefix`, `messageType`), at most
  1000 per call, and answer `{affected, more}`. They are built in `ActorSystem` on the single operations and the
  listings, with no new store methods, and are not one transaction: a call that fails half way keeps what it did.
  Filter-mode retry walks the `failed` outcomes and skips ones already retried (a `causedBy` child), so calling again
  until `more` is false does not send a message twice. Ineligible messages are skipped and not counted.
- **Wire changes.** `POST /messages/{id}/cancel` replaces `DELETE /messages/{id}` (same semantics);
  `GET /actors/{type}/{key}/mailbox` replaces `GET .../messages` and adds `state`; `POST .../messages` stays as send. New
  problem types `no-such-node`, `no-running-turn`, `message-not-retryable`. The path segments `bulk-cancel` and
  `bulk-retry` under `/messages` are reserved. `ActorView` gains `paused`.

## Consequences

- The schema changes in place at version 1 (ADR 0008): the `nodes` table, `actors.cancel_epoch`, `actors.paused`, a
  new partial index `actors_running` and a changed `actors_due`.
- `Store` gets the methods above; every backend implements them and the conformance suite covers them. A store fixture
  must register a type before creating an instance of it.
- Callers of `DELETE /messages/{id}`, `GET .../messages` and the matching client methods move to the new routes;
  `CHANGELOG.md` says so.
- A cancel request takes up to one heartbeat (10 s with the default lease); it has no effect on a node whose handler
  has already ended, and a node restart between the request and the next heartbeat loses the request with the node's
  turn, which then re-runs (at-least-once, G4).
