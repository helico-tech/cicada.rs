# 0034. Best-effort actor change hints

Date: 2026-11-13

## Status

Accepted for `actor-change-feed`. The owner delegated design decisions to an Opus 5.5 adviser.

## Context

A host dashboard needs to know when to refresh actor snapshots. Polling rereads unchanged actors, while a turn
observer sees only local execution and misses peers and operator changes. The state listing of ADR 0033 supplies
resync reads; a bounded feed can supply invalidation hints without a durable event log.

## Decision

- Add ActorSystem::changes(prefix), yielding non-exhaustive ActorChange::Changed { address } and Resync through an
  owned Send + Unpin stream. State/version/kind are absent; reread the address, including deletion/recreation.
- Cover insert, state write, pause/resume and removal/restart, including descendants; exclude mailbox/lease-only
  changes, watches and registry/node metadata. At most one hint per address per transaction, or a coarse Resync.
- Collect at most 100 unique addresses and 7,998 encoded bytes per transaction, then coalesce to Resync. Bound each
  hub to 1024 internal items; lag triggers resync instead of blocking writers. Prefixes are literal address prefixes.
- Memory and SQLite cover only writes through the same Store backend and clones. PostgreSQL covers upgraded writers
  across processes on the same database/schema. Separate SQLite opens and direct SQL writes are outside coverage.
- Subscribe before exposing a ready source; first/recovery Resync comes only when the source is Up. Detected connection
  loss waits for LISTEN to succeed again before resync. Closed is terminal; worker stop leaves the feed open.
- On PostgreSQL's existing cicada_<schema> channel reserve @ payloads: bare @ for resync, otherwise @ followed by
  canonical addresses separated by one space. A malformed batch means resync. Existing type-name wake-ups keep their
  meaning. Batch all pending notifications in the committing transaction, so rollbacks publish no change hints.
- SQLite publishes after COMMIT. A dropped/uncertain transaction is settled under its writer guard before a resync;
  runtime shutdown can prevent timely settlement/publication, requiring a new subscription and full refresh on resume.

## Consequences

- No durable log, replay cursor, exactly-once delivery, global order or snapshot across pages. Resync can be coarse;
  old writers and out-of-scope writes are not detected. Hosts may need periodic reconciliation for those cases.
- Notifications add commit work on PostgreSQL, including state-only commits. Payload bounds avoid format refusal,
  but notification queue exhaustion can fail COMMIT. Map COMMIT-time SQLSTATE 54000 to Unavailable (rather than
  the default class-54 Invalid mapping); other statement/error mappings remain unchanged. Existing retries and
  uncertain acknowledgements apply; this feature is not insulated from infrastructure failure and does not promise unchanged outcomes.
- No SQL schema or actor contract change; G1–G6 and S1–S10 retain their meanings. Watches remain durable actor
  notifications of failure/removal only. TurnObserver needs no extension. SSE transport is a separate change.
- Old PostgreSQL nodes ignore @ notifications but do not publish them. Upgrade all writers, then refresh subscribers.
  The architecture and core/operations guides document the feed's scope, failure boundaries and recovery protocol.

PostgreSQL's [NOTIFY documentation](https://www.postgresql.org/docs/current/sql-notify.html) specifies commit-time
publication, payload bounds and notification queue exhaustion; guards cannot turn that service into an infallible one.

The server [raises SQLSTATE 54000 for queue exhaustion](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/commands/async.c).
The COMMIT-only mapping avoids treating this operational failure as invalid actor input.
