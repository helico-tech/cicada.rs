# Design

## Scope and API

One best-effort core subscription, then SSE in the next change. The owner's Opus 5.5 adviser settled the split and
failure boundaries. Five tasks plus finishing checks. ADR 0034 defines the inter-node format; no SQL schema change.
Add change.rs, root exports ActorChange and ActorChanges, and ActorSystem::changes(&self, prefix: &str) -> ActorChanges.
ActorChange is Debug/Clone/PartialEq/Eq and non_exhaustive, with non_exhaustive Changed { address: ActorAddress } and
Resync. ActorChanges wraps BoxStream<'static, ActorChange>, is Send + Unpin, and owns its receiver/state; next is
cancel-safe. Call changes inside a Tokio runtime; no worker start or hosted type is required. There is no error item.
Read the address after Changed (None removes it); replace the view by paging list_actors_with_state after Resync,
then process buffered hints. Versions reset on recreation, so events carry neither version, state nor operation kind.
Not chosen: TurnEnd extension (local turns miss operator changes and peers), replay log/outbox (schema and lifecycle),
or version comparison as identity. Watches still report only failures/removals to actors, not general state changes.

## Coverage and bounded collection

Report inserted actors (operator/turn/child creates), NewState including equal-value writes, pause/resume, restart,
and removal by operator delete, turn delete or stop including descendants. Deduplicate addresses per transaction.
Idempotent create and absent operations emit nothing; repeated pause/resume may emit a harmless hint. Unchanged-state
turns without these effects, mailbox-only changes, leases, watches, registry/node rows and sweeps emit no Changed.
Rollbacks/refusals/LeaseLost emit no Changed. A failed handler's effects are discarded; its committed watch-message
sends alone do not count. Hints are eligible only after commit; a later read can already see a subsequent commit.
A shared collector keeps at most 100 unique canonical addresses and an encoded payload of at most 7,998 bytes,
including @ and separators. Crossing either bound clears it and makes overflow sticky. If a mutation cannot supply
its affected addresses from work it already does, mark overflow instead of adding a discovery query. Empty emits
nothing; overflow emits Resync; otherwise publish the collected addresses once. No cross-transaction ordering promise.
The literal address prefix is filtered at receipt; empty matches all, `order` includes `orders/x`, `order/` does not.
Resync reaches every prefix. Duplicates and conservative Resyncs are allowed; hints contain no history or durable cursor.

## Hub, readiness and lifecycle

Each store owns a bounded broadcast hub (1024 internal items) with Up/Down/Closed source state. A short std mutex
serializes state transitions, publication and subscribe-plus-state-snapshot; never hold it across await. Memory and
SQLite start Up. PostgreSQL starts Down and becomes Up only after LISTEN succeeds; the shared listener starts from
changes even when the worker never starts. Subscribe synchronously before returning ActorChanges, under the hub lock.
A pending initial/recovery Resync establishes a fresh receiver and reads current source state atomically under that
lock: if Up, yield Resync; if Down, wait; if Closed, end. Discard old queued hints covered by that resync boundary.
Thereafter yield matching Changed items, process Down by waiting, and process Up or a Resync hint by scheduling the
same recovery boundary. Lagged also schedules that boundary. Preserve source order for already yielded hints; a queued
Changed before Down may still be yielded. Unknown disconnects can leave stale Up temporarily, but detection followed
by a successful reconnect yields another Resync. No claim of instant loss detection or a global read snapshot.
Poll work must be bounded when skipping irrelevant items; yield to Tokio rather than draining an endless hot source.
Closing is terminal and wins over buffered items: check Closed on poll, close the hub, wake waiters and discard backlog.
Close before first poll, or subscribe after close, ends without Resync. Serialize listener start/stop so a racing start
cannot revive Closed. System::stop does not close the store/feed. Dropping a stream frees only its receiver; the shared
listener remains until Store::close. While PostgreSQL cannot connect, no first item arrives: hosts may set a timeout.
Not chosen: immediately yielding startup Resync before LISTEN (missed-change gap), or ending on lag (unneeded reconnect).

## Memory and SQLite publication

Memory collects under its state mutex, then publishes synchronously after unlocking with no await between them.
SQLite attaches the collector to WriteTx; confirmed COMMIT publishes synchronously before returning. Known rollback
emits nothing. An error/uncertain ending after COMMIT was attempted emits Resync after transaction settlement.
Use an Arc writer mutex and OwnedMutexGuard in WriteTx. A cancelled/dropped transaction with collected changes moves
its guard into a settle task: queue rollback behind outstanding work on that connection, await it, publish Resync,
then release the guard. This covers a COMMIT sqlx executes after its caller is dropped and prevents the next writer
from overtaking settlement. Track completion/commit phase so successful ends do not trigger Drop's fallback.
If no running runtime can complete settlement (including runtime shutdown), no timely post-settlement hint is promised;
retain existing rollback-on-next-use behaviour and require a fresh subscription/full refresh when service resumes.
A fallback immediate Resync must not be documented as proof of settlement. Process death loses the local feed anyway.
Coverage is only the same Store backend and its clones, not separate opens of the same SQLite file, even in one process.
Not chosen: data_version polling (coarse invalidations for unrelated writes), triggers/log tables, or cross-process SQLite.

## PostgreSQL publication and listener

Keep channel cicada_<schema>. One feed payload per transaction: @ plus canonical addresses separated by single spaces;
bare @ means Resync. Addresses contain neither literal spaces nor @. Reserve all @ payloads. Parse the whole batch
before yielding any item: invalid/noncanonical token, leading/double/trailing space or unknown @ format means Resync.
Ordinary type-name wake-ups, probes and empty strings are ignored by the feed. Wake-up consumers ignore @ payloads.
Flush existing wake-up payloads and the optional feed payload together with one parameterized pg_notify/unnest statement
before COMMIT. All changed actor paths, including create-and-lock/restart, must collect within that attempt. Retried
transactions get a fresh collector; the server delivers only committed notifications, even after a lost acknowledgement.
The listener publishes typed Up/Down/Payload events. Set PgListener::eager_reconnect(false), then treat try_recv
Ok(None) as connection loss and reconnect through our own loop; publish Up only after the new LISTEN completes, not when sqlx merely notices loss. Wake-ups
wake on Up and lag; preserve their type filtering. Shared 1024-item capacity includes ordinary wake-ups, so these can
cause feed resyncs. Close aborts/joins listener work, closes the hub and prevents restart; idle feeds hold no DB transaction.
The size/count guard avoids invalid payloads, NOT infrastructure failure. State-only commits now notify too: extra
commit work/notification locking and a full PostgreSQL notification queue can fail COMMIT. PostgreSQL uses SQLSTATE
54000 here; at the COMMIT statement only, map 54000 to Unavailable instead of the current class-54 Invalid mapping.
Keep other error mappings and transaction retry classification unchanged; ordinary executor retry/lease rules apply.
Timeouts/acknowledgement uncertainty remain possible. Do not claim outcomes are unaffected.
Not chosen: postcommit publication (undetectable crash gap) or an outbox (durable machinery beyond this small feed).

## Compatibility and verification

PostgreSQL covers upgraded writers on the same database/schema; old writers and direct SQL edits produce no feed hints.
Old wake-up consumers ignore @ payloads. Upgrade all writers and refresh subscribers afterward; no schema/restart barrier.
Memory/SQLite external writes and runtime shutdown gaps are not detected. This is a refresh hint, not a correctness
boundary; hosts needing reconciliation beyond this scope retain a periodic or explicit full refresh path.
Use deterministic virtual-time hub/runtime tests, conformance on all backends, SQLite driver barriers for cancellation,
and gated PostgreSQL cross-store/listener-loss tests. Verify rollback/empty effects, all row-write paths, descendant
removal, coalescing, prefix grammar, startup/lag/reconnect/close races and wake-up compatibility; run no benchmarks here.
Document core examples and architecture, operations (queue usage, cost, scope and upgrades), changelog and version.

Review 1: ready for tasks. Disabled eager reconnect to avoid a redundant sqlx connection cycle; added the closed-store
stream exception to the store spec/doc work and phase-specific SQLite cancellation tests.
