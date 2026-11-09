---
status: done
created: 2026-10-07
---

# 07. Port the runtime test suite of `actors-test` (dispatch, leases, retry, hooks, outcomes, timeouts, shutdown)

## Goal

Port the runtime test suite of `actors-test` (dispatch, leases, retry, hooks, outcomes, timeouts, shutdown) and the
core unit tests of `actors-core`, and fix the runtime deviations from Kotlin semantics they reveal.

## Acceptance criteria

- Every Kotlin runtime test has a Rust counterpart or a recorded reason why not.
- Core unit tests that pin behaviour of the Rust design are ported; checks the Rust type system makes impossible are
  listed as covered by the compiler.
- `cargo test -p cicada -p cicada-test` and `cargo clippy -p cicada -p cicada-test --all-targets` are clean.

## Plan

1. Shared fixtures in `crates/cicada-test/tests/common/mod.rs`: the Kotlin `Fixtures.kt` (counter, echo) as Rust
   actor types, a `Faults` wrapper store (the Kotlin `DelegatingStore`) with per-test overrides, a virtual-time node
   builder and small helpers.
2. One test file per Kotlin test class in `crates/cicada-test/tests/<topic>.rs`; paused tokio time where Kotlin uses
   `runTest`/`testActors`, real time (multi-thread runtime where a handler blocks a thread) where Kotlin uses
   `runBlocking`.
3. Core unit tests as integration tests in `crates/cicada/tests/` (those that need a node in `cicada-test`).
4. Fix the runtime where a ported test shows a deviation; log each fix here.

## Port map

| Kotlin | Rust | Tests (Kotlin → Rust) |
| --- | --- | --- |
| `TurnOutcomesTest` | `cicada-test/tests/turn_outcomes.rs` | 18 → 18 |
| `UnhandledTest` | `unhandled.rs` | 5 → 6 (`drop` as policy and as a custom decision) |
| `HooksTest` | `hooks.rs` | 5 → 5 |
| `HookSemanticsTest` | `hook_semantics.rs` | 13 → 12 |
| `TurnCancelTest` | `turn_cancel.rs` | 4 → 4 |
| `TurnTimeTest` | `turn_time.rs` | 3 → 3 |
| `MessageRetryTest` | `message_retry.rs` | 22 → 22 |
| `MessageChainTest` | `message_chain.rs` | 6 → 6 |
| `MessageRetentionTest` | `message_retention.rs` | 2 → 2 |
| `OperationsTest` | `operations.rs` | 6 → 6 |
| `StartupTest` | `startup.rs` | 8 → 7 |
| `TypedEffectsTest` | `typed_effects.rs` | 5 → 5 |
| `LeaseTest` | `lease.rs` | 4 → 4 |
| `LeaseRobustnessTest` | `lease_robustness.rs` | 6 → 6 |
| `AfterHandlerTest` | `after_handler.rs` | 12 → 11 |
| `TurnStructureTest` | `turn_structure.rs` | 3 → 3 |
| `OverrunTest` | `overrun.rs` | 5 → 5 |
| `DispatchLoopTest` | `dispatch_loop.rs` | 10 → 10 |
| `ShutdownAndIsolationTest` | `shutdown_and_isolation.rs` | 3 → 3 |
| `ArchitectureExampleTest` | `architecture_example.rs` | 2 → 3 |
| `VirtualClockTest` | `virtual_clock.rs` | 1 → 1 |
| `ActorTypeTest` | `cicada-test/tests/actor_type.rs` (needs a node) | 7 → 5 |
| `AddressingTest` | `cicada/tests/addressing.rs` | 17 → 17 |
| `RetryPolicyTest` | `cicada/tests/retry_policy.rs` | 6 → 6 |
| `CanonicalJsonTest`, `JsonSchemasTest`, `MessageCodecTest` | `cicada/tests/serialization.rs` | 3 + 6 + 6 → 13 |
| `RegistrySupportTest`, `StoreSupportTest` | `cicada/tests/store_support.rs` | 3 + 5 → 8 |

Not ported, and why:

- Checked by the compiler (ADR 0003): `StartupTest` missing handler / missing unhandled; `ActorTypeTest` missing
  handler, duplicate handler, missing or doubled unhandled, missing template, a message field named `type` (serde:
  "variant field name `type` conflicts with internal tag"); `MessageCodecTest` non-serializable class.
- No Rust counterpart: `HookSemanticsTest` invalid plugin configuration (a Rust plugin is a value its caller builds;
  its constructor refuses a bad configuration); `AfterHandlerTest` `onOutcome hooks run on the handler context` (a Rust
  node has no `handlerContext`); sealed sub-families in `ActorTypeTest`/`MessageCodecTest` (families are flat enums).
- Different generator (ADR 0004): the `JsonSchemasTest` class snapshot; `actor_type.rs` pins the schemars snapshot
  instead.
- Other stories: `RetryPolicyTest`'s `MessageView` checks (HTTP wire model, story 08); `InMemoryStoreConformanceTest`
  (04); `ContractSnapshotsTest`, `StateSamplesTest` (14).

Adaptations: Kotlin error texts that are exception `toString()`s (`java.lang.IllegalStateException: kaput`) are the
error's own text in Rust (`kaput`). A handler that "ignores cancellation" (Kotlin `NonCancellable`) blocks its thread in
Rust, since a future cannot ignore being dropped. Kotlin's `job.cancel(); job.join()` on `runDueTurns` is
`JoinHandle::abort()` plus a short wait (`settle()`), as the release of a dropped turn happens in the background.

## Progress log

- 2026-10-07: started; read the Kotlin tests and runtime sources.
- 2026-10-07: fixtures and the `TestActors`-based tests ported; all passed without runtime changes.
- 2026-10-07: lease, after-handler and turn-structure tests showed four deviations, fixed in
  `crates/cicada/src/runtime/system.rs` and `executor.rs`:
  1. Dropping `run_due_turns` left its turns running detached and never released their leases (dropping a tokio
     `JoinHandle` does not abort the task). Turns now run as tasks under a cancellation token that a drop guard
     cancels, the same path as a node stop after its grace period.
  2. A cancelled turn released its lease before its worker had stopped. It now aborts the worker and waits for it
     (Kotlin's structured concurrency), then releases only when the handler did not end.
  3. Claims answered after their caller was gone (a dropped `run_due_turns`; a dispatch loop that `stop` left behind)
     were dropped and their leases left to expire. A `ClaimBatch` now releases them.
  4. A worker that ended only after its lease was lost (code that does not yield) had its decision committed; it is
     now abandoned, as Kotlin does.
- 2026-10-07: real-time tests (overrun, dispatch loop, shutdown) and the core unit tests ported; all pass. Clippy clean.

## Follow-ups

- [0200 `RetryPolicy` and `Retention` have no JSON Schema](../../issues/0200-policies-without-json-schema.md)
