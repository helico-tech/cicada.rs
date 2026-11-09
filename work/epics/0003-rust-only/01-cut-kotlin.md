---
status: done
created: 2026-11-09
---

# 01. Cut the Kotlin contract

## Goal

Nothing in this repository depends on, or promises anything to, the Kotlin implementation.

## Acceptance criteria

- ADR 0008 is accepted; ADR 0001 and 0004 are marked superseded.
- `kotlin_schema.rs` and the `kotlin-*.json` state samples are gone.
- `PostgresConnection::parse` refuses `jdbc:postgresql:` URLs, with a test.
- The console's wire types are no longer described as generated: `generated.ts` becomes `wire.ts`.
- `README.md`, `AGENTS.md`, `docs/architecture.md`, `docs/operations.md`, `e2e/README.md` and code comments describe
  this library without Kotlin. Test names no longer mention Kotlin.
- The stored and wire formats are unchanged (existing tests on them still pass with the same values).

## Plan

1. Delete the Kotlin-only test and samples, remove JDBC parsing.
2. Rename `generated.ts` to `wire.ts` and update its header and imports.
3. Remove "(Kotlin `XTest`)" notes and other Kotlin wording from comments; rename tests.
4. Rewrite the docs.
5. Run fmt, clippy, `cargo test --workspace`, the PostgreSQL tests and the console tests.

## Progress log

- 2026-11-09: Removed the Kotlin-only test, samples and JDBC URLs; `generated.ts` is now `wire.ts`; comments and docs
  rewritten. `redact` now leaves out the query string, because a refused JDBC URL would otherwise print its
  `?password=`. fmt, clippy, `cargo test --workspace` with PostgreSQL and the console tests pass. E2E and
  `CICADA_DOCKER` tests not run.

## Follow-ups

- [0900](../../issues/0900-connect-options-error-url.md): `connect_options` error can print a URL password.
