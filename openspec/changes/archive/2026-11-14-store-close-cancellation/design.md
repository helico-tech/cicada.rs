# Design

## Closed is not cleanup complete

PostgreSQL and SQLite currently set closed before awaiting cleanup, then every later caller returns early. A caller
cancelled at an await can leave connections retained. Fix both with a private resumable gate; memory close already
finishes synchronously. Rename the proposal to store-close-cancellation, retaining its queue position. This is one
resource-lifecycle repair, not an actor stop, migration, connection timeout or background-task framework.

Creating an unpolled close future does nothing. On every polled SQL close call, before any await, idempotently set
closed, close the change hub and stop new resource admission. PostgreSQL synchronously requests listener stop and
calls/drops pool.close() to mark the main pool closed; SQLite closes reader_permits. Repeat this short prefix for
every caller, rather than guarding its side effects with closed.swap: on different threads another caller could
otherwise overtake the first between its flag update and side effects. All prefix operations are idempotent. Once a
call reaches its first await, new operations are unavailable and changes are terminal; a never-polled call has no effect.

Then every caller locks a tokio mutex containing retained progress. If done, return; otherwise run the unfinished
cleanup and set done last. A cancelled caller releases the gate without setting done. The next caller resumes; no
caller returns early merely because closed is true. No cleanup runs in the background on behalf of this new gate.
With handles alive and no later close, unfinished cleanup can remain until another close or final drop: document it.

## PostgreSQL

Use Closing { listener: Option<JoinHandle<()>>, done: bool } behind the gate. Split NotificationListener stop into a
synchronous request that sets stopped and aborts while retaining its handle, and take_task that sets stopped, aborts
and transfers that handle. Both use its existing std mutex, excluding start. In the gate, take the task if needed,
await a mutable reference to the stored handle, then clear it; cancellation must not drop the handle being awaited.
Ignore JoinError as today. The predecessor's scoped probe drops with Run. Finish with pool.close().await, then done.

sqlx 0.9's Pool::close marks the pool closed when called, before its returned future is polled. Explicitly drop that
future in the prefix, with a comment and a narrowly justified lint allowance only if necessary. The gate calls it
again to wait for checked-out connections and close idle ones. Its implementation supports repeated calls; canceling
it drops a currently owned connection and leaves remaining idle connections for the next call. Preserve this behavior
when updating sqlx. Marking the pool closed stops maintenance/acquisition and closes returned connections.

The stop request remains effective if every close caller goes away: Run is aborted even if nobody awaits its retained
handle. Dropping the backend drops the remaining handle and pool; it does not create a new Cicada shutdown task or
an Arc cycle. Driver-owned UNLISTEN/connection-return work may still finish later, as in listener-probe-on-close.

## SQLite

Use a tokio mutex holding done. Under it, await writer.lock, take the writer and close it; then pop idle readers one
at a time under the readers std mutex and close each outside that lock. Do not take the whole vector before an await.
The writer slot/readers vector retain untouched connections across cancellation. A connection taken for its own close
is owned by that future; if cancelled it drops and its sqlx worker closes it asynchronously. Do not claim that a later
close joins that already-dropped worker. Retrying waits for the remaining connections, then sets done.

Move Reader::drop's closed check inside the readers mutex before deciding to push. A return either enters the pool
before the close drain takes the lock, or observes closed and drops its connection; it cannot refill a drained pool.
Keep the existing semantics for operations admitted earlier: close waits for the writer/settle guard but does not
wait for reads already holding a connection. Such a reader drops its connection on return. This is not forced rollback.

## Completion and alternatives

All concurrent callers serialize through the gate and observe done only after the retained cleanup steps complete.
PostgreSQL still waits for borrowed main-pool connections; SQLite waits for its writer but not active reads. Neither
backend promises a shutdown duration, immediate socket closure, or joining driver cleanup from previously dropped
connection futures. Hosts may time out a close and call it again. ActorSystem::stop and the App shutdown order stay.

No worker runtime is captured by a new shutdown task; awaiting a retained listener handle uses its existing runtime
and termination result. Runtime shutdown and final Store drop use existing Tokio/sqlx destruction rules, not a promise
of graceful cleanup after the runtime has gone. Expected listener join/connection close errors keep existing handling.
Do not promise recovery from arbitrary panics or resurrect resources after a driver panic; done is written only last.

A spawned shared shutdown job adds lifetime/runtime management without a need: resumability satisfies the proposal.
OnceCell initialization can be cancelled too; a retained boxed future needs extra ownership. An early-return closed
flag cannot express completion, and repeating cleanup without a gate loses the listener join handle between callers.
No public signature, schema, wire or G/S guarantee changes, so no ADR. Document the clarified close contract in
Store::close, Backend::close, architecture and operations; patch release/changelog are chosen when built.

## Proofs and scope

SQLite tests block the writer with the existing commit fault, poll close to Pending, cancel, release the blocker and
verify a second close empties writer/readers. A second concurrent closer must remain pending until the writer is free.
Do not try to inspect writer contents by awaiting its lock while deliberately holding it: inspect after releasing the
blocker. Test never-polled close and a reader held across close returning to no pool. Inspect the lock-protected
return decision and cover concurrent return/drain with controlled synchronization where useful, not timing sleeps.

PostgreSQL tests start LISTEN, hold a main-pool connection and leave an idle one, poll close to Pending and cancel.
Assert pool.is_closed, unavailable operations and terminal streams. A second closer stays pending while the borrowed
connection is held, then finishes after release; assert main-pool size zero and repeated completion. Retain the
listener handle through a cancelled join and test that a later caller observes its completion. Use minimal private
test access, barriers/polling and real-PostgreSQL bounded observation, not arbitrary sleeps. Never-polled calls stay open.

Run cancellation/concurrency regressions against old code first where applicable. Use virtual time for in-process
gate/actor tests and existing real-SQL harnesses for driver I/O. Keep no-feature/memory behavior, change-feed closure,
SQLite settle tests and the predecessor's listener PID regression green. Five tasks plus finishing suffice. Later flow
proposals are outside this request; only their predecessor reference changes with this rename.
