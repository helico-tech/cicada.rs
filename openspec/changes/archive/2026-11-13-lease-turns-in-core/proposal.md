# Proposal

## Why

Remote actor execution needs a public way to acquire and keep a turn's lease. The Backend trait stays private;
HTTP handlers and workers must use a deliberate core API rather than expose database operations directly.

## What Changes

- Add public lease, renewal and release operations over ActorSystem, with public core lease results.
- Return the claimed message, state, version, epoch and store times; record the worker as lease owner.
- Reuse store exclusivity and cancellation signals; apply the server's claim limit before handing out work.
- Define caller cancellation and late claims, repeated release, worker identity and deadline responsibilities.

## Settled decisions

- Address and epoch identify renew/release; trusted workers use any server on the shared database.
- Server NodeConfig grants TTL and claim-limit policy; a selected name/hash map is checked before every claim.
- Renewal atomically retains the later expiry across all backends, so a shorter peer TTL cannot invalidate a
  holder's previously confirmed deadline while its response is lost.
- Guard unhanded claims through filtering, check the deadline again before return, and document failed cleanup.
- At-limit claims get one failure-commit attempt and release fallback; no handler runs, and local policy is unchanged.
- No public transport trait, HTTP types, node heartbeat, migration or server-local lease session in this change.

## Capabilities

### New Capabilities

- `turn-leases`: public core lease/renew/release operations and caller responsibilities.

### Modified Capabilities

- `claim-limit`: enforce the serving system's limit before external handoff.
- `store-backends`: renewal never shortens a held lease, including under concurrent peer renewals.

## Out of scope

Completion reports, HTTP lease endpoints, worker dispatch, durable claim receipts, worker node-list heartbeats,
long-polling, per-type authorization, and a new store backend interface.

## Order

After `register-contracts-over-http`, before `complete-leased-turns`.

## Impact

Public core lease API/results, claim cleanup and policy, atomic renewal in all backends, virtual-time runtime and
conformance tests, an ADR for remote turn semantics and the core guide. No Backend signature or SQL schema change.
A lease can be inspected, renewed and released before completion is added next.
