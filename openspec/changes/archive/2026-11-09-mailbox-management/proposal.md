# Proposal

Status: built on 2026-11-09.

## Why

An operator can list a mailbox and cancel one message at a time, but cannot hold an actor still while looking into a
problem, or clear its mailbox without deleting the actor. The listing also does not show which message is running.

## What Changes

- **M1.** `GET /actors/{type}/{key}/mailbox`: each item is marked running or pending.
- **M2.** `POST /actors/{type}/{key}/mailbox/pause` and `/resume`. A paused actor is not claimed; messages still queue
  up. A running turn finishes.
- **M3.** `POST /actors/{type}/{key}/mailbox/purge`: every pending message ends `purged`, the actor stays. A message
  being handled is left alone. Returns the number purged.

## Out of scope

- Reordering messages. No source showed operators relying on it, and it would weaken G5.
- Pausing a whole type or a whole node.

## Impact

- A new column on actors, a change to `claim_due` and `next_due_at`, new store methods in all three stores and new
  conformance cases. The schema changes in place at version 1 (ADR 0008).
- S8 (liveness) gets an exception for paused actors, so this needs an ADR and an update of `docs/architecture.md`.
- `cicada-axum`, `cicada-client`.
