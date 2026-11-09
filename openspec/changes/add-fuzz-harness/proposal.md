# Proposal

## Why

Every test today is a scenario someone thought of. Cicada's value is that G1–G6 hold under crashes, lost
acknowledgements and outages, and the orderings that break those guarantees are the ones nobody imagined. There are no
randomised tests yet.

## What Changes

A new test-only crate runs three kinds of randomised test, built in this order. Work stops for a review after step 2.

1. **Input fuzz.** Random text into the address, percent-encoding, duration, instant and cursor parsers, and into the
   HTTP request bodies. Parsers must not panic, and what they format they must parse back.
2. **Backend comparison.** The same random list of store operations runs on `InMemoryStore` and `SqliteStore` on a
   virtual clock. Their results must match. This also shows whether `TestActors` tests can be trusted for real stores.
3. **Node simulation.** Several real `ActorSystem`s run on one thread and paused tokio time. Each talks to one shared
   `InMemoryStore` through its own fault wrapper, which can fail a call before it applies, fail it after it applies (a
   lost acknowledgement), slow it down, or go dead for good (a node crash). After the faults stop and the mailboxes
   drain, an event log is checked against G1–G6 and S8.

Also:

- The three unbiased `tokio::select!` calls in the runtime get `biased;`, so that a failing scenario replays the same
  way. This is the only production code change.
- A failing case shrinks to a short list of steps. The cases that failed are committed, so a found bug stays tested.
- A short run is part of `cargo test`. Longer runs are on demand, with `PROPTEST_CASES`.

Bugs the harness finds are not fixed in this change. Each becomes its own change with only a `proposal.md`.

### Out of scope

- PostgreSQL. It has no virtual clock, so it cannot be compared exactly, and its main risk is real concurrency. That
  needs its own test.
- The node simulation on SQLite. sqlx does real I/O on a worker thread, and paused tokio time jumps ahead while it
  waits, so a run would not be repeatable.
- Real multi-threaded concurrency on any backend.
- Coverage-guided fuzzing (`cargo-fuzz`), which needs nightly and sits outside `cargo test`.
- Asserting that `on_outcome` runs after a lost acknowledgement. Nothing promises it, and today it does not.
- Any change to a guarantee, the store trait or a wire format.

## Capabilities

### New Capabilities

- `randomized-testing`: how randomised tests exercise the parsers, the store backends and the runtime, what they check,
  and how a failure is replayed and kept.

### Modified Capabilities

None.

## Impact

- New crate `crates/cicada-fuzz` (test-only, not used by any library crate) and a workspace dev dependency on
  `proptest`.
- `crates/cicada/src/runtime/executor.rs` and `crates/cicada/src/runtime/system.rs`: `biased;` on three `select!` calls.
  No public API changes.
- `cargo test --workspace` takes somewhat longer, and CI with it.
- `AGENTS.md` and `docs/architecture.md`: the crate is added to their lists, with how to run a long fuzz.
