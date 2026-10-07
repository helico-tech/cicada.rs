---
status: in-progress
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
3. Core unit tests as integration tests in `crates/cicada/tests/`.
4. Fix the runtime where a ported test shows a deviation; log each fix here.

## Progress log

- 2026-10-07: started; read the Kotlin tests and runtime sources.

## Follow-ups

- (none yet)
