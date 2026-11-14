# Proposal

## Why

PostgresStore::close sets its closed flag before awaiting listener.stop and then the main pool's close. If its caller
is cancelled during that sequence, shutdown can stop before the main pool is closed. A later close sees the flag and
returns immediately instead of finishing the work. The listener-probe-on-close investigation found this separate
shutdown boundary; owning the probe inside the listener does not resolve main-pool cleanup.

SQLite has the same early-return bug while waiting for its writer, and its reader-return check can race the reader
drain. This change is named `store-close-cancellation` to cover the same resumable shutdown behavior on both SQL
backends. The in-memory close has no await and needs no implementation change.

## What Changes

- Make SQL store shutdown resumable after a cancelled close, without allowing new store work or
  reopening the change hub.
- Define how repeated/concurrent callers observe shutdown completion, and prove cancellation at the relevant awaits
  does not permanently skip pool/connection cleanup. Serialize cleanup with a retained progress gate; start no new
  shutdown task. Protect SQLite reader returns against refilling the closed reader pool.

## Decisions

- Repeated callers resume unfinished cleanup, and concurrent callers await the same completion boundary.
- Closure takes effect on first poll before any await; never-polled close futures have no effect.
- Active operations retain existing behavior and close has no time bound. Driver-owned cleanup and SQLite reads
  already in flight may outlive the awaited cleanup; the design states the limits.

## Out of scope

The listener's probe ownership (the preceding fix), reconnect policy, wire formats, schema changes and actor execution.

## Order

After `usage-error-password-redaction`, before `flow-step-sends`.

## Impact

PostgreSQL and SQLite close/lifecycle state, listener stop ownership and targeted cancellation tests. No public
signature, SQL schema or wire change is proposed.
