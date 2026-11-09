---
status: done
created: 2026-10-07
---

# 13. Worked examples with tests

## Goal

Worked examples with tests.

## Acceptance criteria

- Every Kotlin example and its test ported.

## Plan

Port `examples` from the Kotlin repo into the crate `cicada-examples`, one module per example, one integration test
file per example under `examples/tests/`.

- Ten examples: `singleton` (Cleanup), `timer` (Checkout), `cron` (CronJob), `reply` (Cart, Pricing), `saga` (Order,
  Inventory, Payments), `steps` (Onboarding), `fanout` (Batch, BatchItem), `registry` (DeviceRegistry, Device),
  `supervision` (Mailer, with the extras' DeadLetters and FailureWatcher), `sync` (Sync, Reports).
- Keep names (snake_case), templates, message type names, state shapes (camelCase JSON, enum values as in Kotlin) and
  the comments that explain each pattern (as module docs).
- Rust shape: a public ref struct per type, a message enum with `#[serde(tag = "type")]`, a public actor struct whose
  fields are its dependencies (fake external services in tests), one `impl Actor` with a `match`.
- Tests: `#[tokio::test(start_paused = true)]` with `cicada_test::TestActors`, the same scenarios and assertions as
  the Kotlin tests.

## Progress log

- 2026-10-07: ported the seven examples that need only the core (singleton, reply, saga, steps, fanout, registry,
  sync) with their 22 tests.
- 2026-10-07: merged main (cicada-extras); ported timer, cron and supervision with their 11 tests. All 33 tests and
  the doctest pass; clippy is clean.
- 2026-10-07: the saga passes `cicada_otel::trace_headers(turn)` on every send, as the Kotlin saga does.
- Deviations from Kotlin, on purpose:
  - Pricing replies with its own `PricingReply` enum (`Quote`, `QuoteRejected`); the Cart's family has variants of the
    same names and fields. Kotlin reuses the Cart's classes; in Rust that would make Pricing depend on the Cart's type.
  - External calls that can fail in the tests return a `Result` with a small error type (`GatewayUnavailable`,
    `ServiceError`, `SmtpError`, the cron's `TaskError`); the others (`TempFiles`, `ItemProcessor`, `SyncApi`,
    `PriceList`) are infallible, as their Kotlin signatures and fakes are.
  - `CHARGE_RETRY` and `TICK_RETRY` are functions (`charge_retry()`, `tick_retry()`): the policy constructors
    validate and are not `const`.
  - The cron records the task's error as its `Display` text (`report service down`), not the Java
    `toString()` (`java.lang.IllegalStateException: report service down`). A panicking task fails the tick, which
    its retry policy re-runs, like a cut-off turn.
  - The timer's comment no longer says a handler cannot cancel a message (`turn.cancel` exists); the pattern is the
    same.

## Follow-ups

- [0700 `TestActors` has no `list(prefix)`](../../issues/0700-test-actors-list.md)
- [0701 `Timestamp` and `MessageId` in state need manual schema and serde attributes](../../issues/0701-state-field-schemas.md)
- The contract snapshots of the examples against the Kotlin ones are story 14's job; integer schemas differ
  (schemars adds `format`, e.g. `int32`).
