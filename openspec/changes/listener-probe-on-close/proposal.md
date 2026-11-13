# Proposal

## Why

The PostgreSQL listener starts a probe task for each connection (`crates/cicada/src/store/postgres/listener.rs`,
about lines 103 to 105) and aborts it only after `receive` returns. When the listener's own task is aborted while it
waits in `receive`, the abort of the probe never runs: the probe task keeps going and keeps its pool after the store
is closed. The close test does not see it, because it looks for the channel's text in queries and the probe's queries
are parameterised. Found in the review of `actor-change-feed`.

## What Changes

- Tie the probe task's life to the listener's, so that it ends whenever the listener ends, aborted or not; for example
  an abort-on-drop guard.
- A test that closes the store while the listener waits in `receive`, and checks that the probe task has ended and its
  pool is closed.

## Out of scope

Other changes to the listener, its reconnect backoff or the change feed.

## Order

After `deep-failure-notices`, before `cicada-agent-skill`.

## Impact

`crates/cicada/src/store/postgres/listener.rs` and its PostgreSQL tests. No public API change.
