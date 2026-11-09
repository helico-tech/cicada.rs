# Proposal

Status: built on 2026-11-09.

## Why

An operator cannot see what is running, other than by opening messages one by one, and cannot stop a turn that runs
away. Temporal and Restate both offer a cooperative stop as the normal way.

## What Changes

- **T1.** `GET /turns` (filter by node or by address prefix) and `GET /actors/{type}/{key}/turn`: the running message, the node
  that holds the lease and when the lease expires. The columns `lease_owner`, `lease_until` and `lease_message` already
  hold this. The view is approximate, as any snapshot of running work is.
- **T2.** `POST /actors/{type}/{key}/turn/cancel` asks for the turn to stop. The store records the request. The owning
  node sees it at its next heartbeat, stops the handler and commits the outcome `cancelled`, without the turn's effects.

The owning node does the commit, so the turn stays fenced and no second turn can start meanwhile: G1 holds. It takes
up to one heartbeat (10 s with the default lease). As with every cancellation today, the handler stops at its next
`.await`, and blocking code runs until it returns.

## Out of scope

- Forcing a lease away from a node. Decided against on 2026-11-09: a dead node's lease expires by itself within the TTL,
  and on a live node it would break G1 (overlapping turns) and G6 (re-running a handler that returned).
- Passing a cancellation token into handlers.

## Impact

- `Store::renew` must be able to report a cancel request, a new store method records it, and the conformance suite
  grows. The schema changes in place at version 1 (ADR 0008).
- `runtime/executor.rs` (the keeper), `cicada-axum`, `cicada-client`.
- `docs/architecture.md`: the turn semantics table gets the new case, so this needs an ADR.
