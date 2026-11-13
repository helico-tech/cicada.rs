# Proposal

## Why

A leased actor turn must submit all of its effects in one fenced transaction. The server needs a handler report,
not permission for the client to choose arbitrary message outcomes or bypass retry and validation rules.

## What Changes

- Add completion reports for returned, failed and cancelled handlers to the public lease API.
- Apply state, sends, creates, cancels, watches and controls through the existing atomic commit path.
- Share validation and failure-to-retry decisions with the local executor; derive sender and caused-by identity.
- Define stale epochs, rejected effects, repeated reports, lost acknowledgements and failure fallback.

## Settled decisions

- Returned/Failed/Cancelled reports distinguish unchanged, set and stopped state; all current effects are supported.
- Shared server policy derives outcomes and retry from stored Pending or Running messages, never from caller counters.
- One commit attempt per call; a known rejection returns the exact nonretryable failure report to submit next.
- The caller retains that transition across uncertainty, never restores original effects and never releases after handler end.
- No durable receipts: repeated committed epochs return LeaseLost with explicit acknowledgement uncertainty.

## Capabilities

### Modified Capabilities

- `turn-leases`: report and settle leased turns through the public core API, including explicit rejection fallback.

## Out of scope

HTTP transport, worker scheduling, a guarantee of exactly-once external effects, and a separate public Backend API.

## Order

After `lease-turns-in-core`, before `remote-worker-runtime`.

## Impact

Core completion types/operation, shared executor policy and validation, effect and failure tests, architecture,
guarantee test references and ADR 0038. No Backend or schema change. Reconcile names against the landed predecessor.
