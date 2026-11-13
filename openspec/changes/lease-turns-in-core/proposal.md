# Proposal

## Why

Remote actor execution needs a public way to acquire and keep a turn's lease. The Backend trait stays private;
HTTP handlers and workers must use a deliberate core API rather than expose database operations directly.

## What Changes

- Add public lease, renewal and release operations over ActorSystem, with transport-ready lease views.
- Return the claimed message, state, version, epoch and store times; record the worker as lease owner.
- Reuse store exclusivity and cancellation signals; apply the server's claim limit before handing out work.
- Define caller cancellation and late claims, repeated release, worker identity and deadline responsibilities.

## To decide

- The narrow transport interface and result/error types, without exposing private store internals.
- Requested types, contract compatibility, lease TTL/batch bounds, unknown types and local/remote coexistence.
- Ownership and full-write trust, any-server renewal/release, claim response loss and conservative client deadlines.
- Server observers for claim-limit failures, wake-ups, shutdown and unstarted ActorSystems.
- Whether the protocol requires any durable metadata beyond the existing leases; avoid process-local sessions.

## Out of scope

Completion reports, HTTP lease endpoints, worker dispatch, durable claim receipts, worker node-list heartbeats,
long-polling, per-type authorization, and a new store backend interface.

## Order

After `register-contracts-over-http`, before `complete-leased-turns`.

## Impact

Public core lease API and views, claim cleanup and policy, virtual-time runtime/conformance tests, an ADR for remote
turn semantics and the core guide. A lease can be inspected, renewed and released before completion is added next.
