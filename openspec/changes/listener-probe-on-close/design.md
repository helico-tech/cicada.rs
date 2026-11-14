# Design

## Ownership

Run the probe as a future inside Run::run, alongside receive in tokio::select!, instead of spawning a second task.
probe borrows the attempt's PgPool and channel and returns Infallible: it sleeps, sends pg_notify and logs errors in
its existing loop. Only receive can finish the select. When it finishes, the probe future is dropped before the
attempt's listener and pool leave scope. Keep hub Up/Down, eager-reconnect policy and reconnect backoff unchanged.

This makes the probe's lifetime structural: aborting/dropping Run drops the probe too. The existing stop aborts and
joins Run; NotificationListener::Drop requests its abort. Cancelling the stop caller after that request cannot detach
a second probe task. No extra handle, task tracker, stop signal or public setting is needed.

Keep the five-second sleep before each probe and the ten-second receive silence timeout. A pending probe query or
pool acquisition does not prevent receive from being polled or timing out. A receive error, connection loss or timeout
drops the probe before reconnecting. Tokio's cooperative I/O budget keeps a notification flood from monopolizing the
task; retain the existing flood and reconnect tests. No timer, notification payload or source-state change.

An abort-on-drop JoinHandle guard would request cancellation without joining the probe and would keep a second task
and helper type. A JoinSet adds the same unnecessary ownership machinery. Explicit pool.close().await or a graceful
stop protocol would broaden this fix and could wait for a broken peer; neither is needed to stop the leaked work.

## Cleanup boundary

The listener future owns all Cicada work for one attempt. Dropping that future releases its probe, PgListener and
pool handles. The owning stop call awaits the aborted listener task; do not strengthen concurrent/cancelled public
close calls into a new completion protocol. Start/stop locking and Closed winning over a late Up remain as today.

sqlx may spawn connection-return/UNLISTEN cleanup when the listener or a query is dropped. Those driver tasks can
retain pool internals until cleanup finishes; sessions need not disappear synchronously with the Cicada future.
Do not await that cleanup in the listener. Against a healthy PostgreSQL server, test eventual disappearance with a
bounded observation; do not promise a wall-clock socket-close bound for a broken peer or runtime shutdown. The main
store pool's existing close remains unchanged, so this design makes no new bound on Store::close as a whole.

## Regression proof

Strengthen closing_the_store_ends_its_streams_and_a_later_subscription_and_never_restarts_the_listener in
crates/cicada/src/tests/postgres/changes.rs, or add a sibling while retaining its stream/unavailable assertions.
Use the existing isolated TestDb and a separate test PgListener on its channel. After subscription has observed Up,
wait with a bounded timeout (about 15 seconds) for PROBE_PAYLOAD and capture PgNotification::process_id(): this is the
probe connection's backend PID and proves the probe actually ran. Identify the production listener's PID before close
using its cicada-listener application name and its LISTEN query containing this unique test channel. Require exactly
one and a different PID; the observer connection must not use the production application name.

Close the store while it is receiving notifications, then use eventually with a healthy-server timeout (about ten
seconds) until neither captured PID exists in pg_stat_activity. Query by the saved PIDs, not by the current SQL text:
the probe uses parameters, and the listener's last query can become UNLISTEN. Keep the observer/inspection connections
outside the production pool and release them during test cleanup. Do not alter production application_name or add
test-only hooks. Wait on positive conditions rather than an arbitrary sleep before close.

Run the regression against the unfixed listener first in the builder's isolated checkout and record that it fails
because the probe's PID remains. Then apply the fix and rerun. Preserve feed end, post-close subscription, unavailable
operation and no-restart assertions. Run lost-listener/resync, reconnect wake-up and flood tests too. No fake-task
framework: the database regression exercises the actual leaked pool, and scoped ownership has no extra state machine.

These are existing real-PostgreSQL multi-thread tests gated by CICADA_PG_URL, which CI supplies. Keep their real-time
harness: paused time can auto-advance receive/acquire deadlines while socket I/O is pending. This does not introduce
wall-time actor-runtime tests; tests of actor behavior continue using TestActors and virtual time.

## Artifacts and scope

Update the change-feed spec with the probe lifetime and asynchronous driver-cleanup boundary; retain existing
readiness/close semantics. No ADR: this repairs resource ownership, changing no public API, wire format, SQL schema
or G/S guarantee. Update listener doc comments and changelog/version at build time; no new guide policy is needed.
Two implementation tasks plus finishing suffice, with PostgreSQL conformance included in finishing.

The adjacent ready changes touch other areas; reconcile against the landed predecessor before building. The owner's
instruction to describe this change overrides the design loop's two-ready-change queue limit for this item.
Separately, PostgresStore::close sets closed before awaiting listener/main-pool shutdown; cancellation can leave the
main pool open and a later close returns early. Track that in postgres-close-cancellation, not in this ownership fix.
