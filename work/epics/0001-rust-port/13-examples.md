---
status: in-progress
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

First part (no dependency on `cicada-extras`):

- `singleton` (Cleanup), `reply` (Cart, Pricing), `saga` (Order, Inventory, Payments), `steps` (Onboarding),
  `fanout` (Batch, BatchItem), `registry` (DeviceRegistry, Device), `sync` (Sync, Reports).
- Keep names (snake_case), templates, message type names, state shapes (camelCase JSON, enum values as in Kotlin) and
  the comments that explain each pattern (as module docs).
- Rust shape: a public ref struct per type, a message enum with `#[serde(tag = "type")]`, a public actor struct whose
  fields are its dependencies (fake external services in tests), one `impl Actor` with a `match`.
- Tests: `#[tokio::test(start_paused = true)]` with `cicada_test::TestActors`, the same scenarios and assertions as
  the Kotlin tests.

Second part (after story 12, `cicada-extras`): `timer` (Checkout), `cron` (CronJob), `supervision` (Mailer).

## Progress log

- 2026-10-07: started the first part.

## Follow-ups

- Port `timer/Checkout.kt`, `cron/CronJob.kt` and `supervision/Mailer.kt` with their tests once `cicada-extras`
  (story 12) is in.
