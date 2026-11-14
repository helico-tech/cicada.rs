# Proposal

## Why

PostgresStore::close sets its closed flag before awaiting listener.stop and then the main pool's close. If its caller
is cancelled during that sequence, shutdown can stop before the main pool is closed. A later close sees the flag and
returns immediately instead of finishing the work. The listener-probe-on-close investigation found this separate
shutdown boundary; owning the probe inside the listener does not resolve main-pool cleanup.

## What Changes

- Make PostgreSQL store shutdown finish or be resumable after a cancelled close, without allowing new store work or
  reopening the change hub.
- Define how repeated/concurrent callers observe shutdown completion, and prove cancellation at the relevant awaits
  does not permanently skip main-pool cleanup.

## To decide

- Shared shutdown task/completion state versus a resumable close operation, including last-handle/runtime teardown.
- How to preserve bounded caller behavior with active operations or a broken peer, without promising immediate socket closure.
- Whether another backend has the same defect; inspect before broadening scope or split any distinct fix.

## Out of scope

The listener's probe ownership (the preceding fix), reconnect policy, wire formats, schema changes and actor execution.

## Order

After `usage-error-password-redaction`. No following change.

## Impact

PostgreSQL store close/lifecycle state and targeted cancellation tests; other backends only if investigation confirms
the same boundary. No public API change is proposed.
