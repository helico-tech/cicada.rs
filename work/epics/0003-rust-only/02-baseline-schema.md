---
status: done
created: 2026-11-09
---

# 02. One baseline schema

## Goal

Replace the six inherited migrations with one clean schema at version 1. Decision:
[ADR 0009](../../../docs/adr/0009-one-baseline-schema.md).

## Acceptance criteria

- SQLite and PostgreSQL each have one migration holding the final tables and indexes, without `outcomes.retried_as`.
- The migration mechanism stays, and a database of a newer version is still refused.
- Tests for upgrades from older versions are removed; every other store test still passes.

## Plan

1. Fold the `ALTER TABLE` columns and later indexes into the `CREATE` statements of both stores.
2. Remove `statements_of` and the old-version tests.
3. Update `docs/operations.md` and the README.
4. Run fmt, clippy, `cargo test --workspace` with PostgreSQL.

## Progress log

- 2026-11-09: Both stores now have one migration; `retried_as`, `statements_of` and the old-version tests are gone.
  fmt, clippy, `cargo test --workspace` with PostgreSQL (1024 passed) and the E2E suite (30 passed) pass.

## Follow-ups
