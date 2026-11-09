# Proposal

Status: built on 2026-11-09 (agreed by Arjan on 2026-11-09). Depends on
`management-api`.

## Why

Mailbox depth and the number of actors are not available without paging through everything. `docs/operations.md` notes
that mailbox depth is not a metric because it needs a store query.

## What Changes

- `GET /stats?prefix=` answers the number of actors, the number of pending messages and the delivery time of
  the oldest due message.

## Out of scope

- Exporting these as metrics.
- History or trends.

## Impact

- A new store method in all three stores and new conformance cases. A count over a large table can be slow; throughput
  is not a goal, but the design should say what a call costs.
- `cicada-axum`, `cicada-client`, `docs/operations.md`.
