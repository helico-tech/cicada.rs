---
status: done
created: 2026-10-07
---

# 04. Store conformance suite S1–S10 as a reusable crate, run against InMemoryStore

## Goal

Store conformance suite S1–S10 as a reusable crate, run against InMemoryStore.

## Acceptance criteria

- Every Kotlin conformance case (`StoreConformanceSuite`, chain, reschedule, retention, robustness, turn-cancel suites) has a Rust counterpart.
- `InMemoryStore` passes it.

## Plan

Port `actors-store-conformance` to `crates/cicada-store-conformance`:

- `StoreFixture` (async trait): one fresh store per case with the controls of the Kotlin fixture: `store`, `is_virtual_time`,
  `advance`, `peer`, opt-in `step_back` and `reopen` (capability flags), `close`.
- `FixtureFactory`: what a Kotlin suite subclass supplies: `create(retention)`, `tick`, `long_retention`, plus
  `skip_reason` so a backend without its server (PostgreSQL without `CICADA_PG_URL`) skips every case.
- `Env`: the Kotlin `Env` helpers (`new_actor`, `send`, `claim_one`, `commit_ok`, `all_pending`, `concurrently`, ...);
  `store_test` runs a case body against a fresh fixture with a timeout and closes it.
- One module per Kotlin file; every case is a `pub async fn` named after the Kotlin case in snake_case, generic over
  the factory. Capability-dependent cases skip (print and return) like JUnit `assumeTrue`.
- `store_conformance_tests!(factory)` expands to one `#[test]` per case, each on its own multi-thread tokio runtime
  (peers run in parallel, as Kotlin's `Dispatchers.Default`). A unit test checks that the macro lists every case.
- Run it against `InMemoryStore` (virtual clock that can step back) in the crate's `tests/`.

## Progress log

- 2026-10-07: started; plan above.
- 2026-10-07: all 102 Kotlin cases ported (59 main, 13 robustness, 11 reschedule, 9 retention, 5 chain, 5 turn
  cancel), same names in snake_case, same assertions. Cases are `async fn`s whose body is an async closure passed
  to `store_test` (the Kotlin `storeTest { }`), so an early `return` skips like `assumeTrue`.
- 2026-10-07: `tests/in_memory.rs` runs the suite on a virtual clock that can step back (as Kotlin's
  `InMemoryStoreConformanceTest`); `tests/in_memory_real_time.rs` runs it on the system clock with a 50 ms tick, to
  check the real-time paths the SQL backends will use. Both pass (102 each; S1 and the time-exact cases skip where
  the fixture lacks the capability). The suite found no bug in `InMemoryStore` or the core; a mutation
  (`lease_until > now` → `>=`) is caught by `s4_a_lease_is_valid_strictly_before_lease_until_and_expired_at_it`.
- 2026-10-07: deviations from Kotlin, all in test mechanics: Kotlin's `+300000-01-01T00:00:00Z` (not representable
  in chrono) is replaced by chrono's latest instant, equally out of range; `shuffled()` in the concurrent
  registration case is a deterministic per-peer reordering; every case may take 2 minutes, the serial-workers case
  3, as in Kotlin.

## Follow-ups

- Stories 05 and 06: implement `FixtureFactory` and call `store_conformance_tests!`; PostgreSQL returns a
  `skip_reason` when `CICADA_PG_URL` is unset, SQLite should also run a real-time variant as Kotlin does.
