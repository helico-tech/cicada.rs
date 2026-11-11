# Proposal

## Why

The randomised test `backends_agree` (`crates/cicada-fuzz/tests/backends.rs`) runs one random list of store
operations on `InMemoryStore` and on `SqliteStore` and expects equivalent answers. On 2026-11-11 it failed in the CI
run of the release commit of 0.3.1 (run 43005035476 of `helico-tech/cicada.rs`, first attempt), after 191 lists that
passed:

```
backends differ at operation 13 (Commit { claim: 2, epoch_delta: 0, end: Done, state: Unchanged, … })
  first:  "lease-lost"                                             (InMemoryStore)
  second: {"committed":{"version":0,"sent":[],"cancelled":[]}}     (SqliteStore)
```

Neither store is wrong. The comparison sent the two stores different commits (`design.md`, Context). When two
instances are due at the same instant, the stores hand out their epochs in a different order, which S4 allows. The
harness then applies `epoch_delta` to the raw epoch number, and that picks different claims in the two stores.

Until this is fixed, any CI run can fail on such a list, and every release needs a green CI.

The shrunk list (every send has `payload: 0` and no sender, retry or retention):

```
 0 Enqueue(Send { to: 1, id: 18, due: After(0ns) })
 1 ClaimDue { types: [0, 1], node: 0, ttl: 1µs, limit: 100 }
 2 Enqueue(Send { to: 0, id: 0, due: Now })
 3 Delete { to: 0 }
 4 Enqueue(Send { to: 0, id: 0, due: Now })
 5 Enqueue(Send { to: 0, id: 0, due: Now })
 6 Commit { claim: 0, epoch_delta: 0, end: Rescheduled(AtAhead(1ns)), state: Unchanged, creates: [(0, 0)],
            sends: [Send { to: 0, id: 1, due: After(1ns) }], cancels: [] }
 7 Advance(1µs)
 8 ClaimDue { types: [0, 1], node: 0, ttl: 1µs, limit: 100 }
 9 SetPaused { to: 1, paused: true }
10 Advance(1µs)
11 ClaimDue { types: [0, 1], node: 0, ttl: 1µs, limit: 100 }
12 Commit { claim: 0, epoch_delta: -1, end: Done, state: Unchanged, creates: [], sends: [], cancels: [] }
13 Commit { claim: 2, epoch_delta: 0, end: Done, state: Unchanged, creates: [], sends: [], cancels: [] }
```

proptest printed this line for the regressions file `crates/cicada-fuzz/tests/backends.proptest-regressions`:

```
cc 094e9ed80e885a42303339923084aba8047c173ed5848559b5dcf032d32033a1
```

## What Changes

- In the comparison (`crates/cicada-fuzz/src/compare.rs`), an `epoch_delta` no longer adds to the raw epoch. It picks
  the epoch seen before or after the claim's epoch for the same instance.
- The shrunk list above becomes a test of its own, and its `cc` line goes into the regressions file.

The simplest version: the fix is in the test crate only. No store changes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `randomized-testing`: the comparison does not depend on the order in which a backend hands out epochs to different
  instances.

## Out of scope

- A tie-break in the claim queries of the stores. The order of claims that are due at the same instant stays free.
- The order of checks in the PostgreSQL commit (`design.md`, decision 2).
- Other differences that the harness may find later; each is its own change.
- How many lists a normal test run tries.

## Order

The first one: until it lands, any CI run can fail on such a list. Before `every-actor-has-state`. It is not a step of
ADR 0018.

## Impact

- `crates/cicada-fuzz` only, and `CHANGELOG.md`.
- No library crate, no store, no wire format and no guarantee changes, so no ADR. Not breaking.
