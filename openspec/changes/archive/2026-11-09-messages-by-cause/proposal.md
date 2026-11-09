# Proposal

Status: built on 2026-11-09 (agreed by Arjan on 2026-11-09). Depends on
`management-api`.

## Why

The message chain lookup was removed in 0.2.0 (ADR 0012). A tool can still follow a message upwards through `causedBy`,
but it cannot find what a message caused.

## What Changes

- `GET /messages/{id}/caused?cursor=&limit=` lists the messages sent by the turn that handled `id`: pending ones and
  retained outcomes, one level down. A tool that wants a tree calls it again per child.

## Out of scope

- The full chain walk that was removed, with its depth and width limits.

## Impact

- A new store method in all three stores and new conformance cases. Brings back what 0.2.0 dropped: the indexes on
  `caused_by` and the column `outcomes.caused_by`. The schema changes in place at version 1 (ADR 0008).
- `cicada-axum`, `cicada-client`.
