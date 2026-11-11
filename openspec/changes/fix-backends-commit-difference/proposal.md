# Proposal

## Why

The randomised test `backends_agree` (`crates/cicada-fuzz/tests/backends.rs`) runs one random list of store
operations on `InMemoryStore` and on `SqliteStore` and expects the same answers. On 2026-11-11 it found a list where
they differ, in the CI run of the release commit of 0.3.1 (run 43005035476 of `helico-tech/cicada.rs`, first attempt),
after 191 lists that passed:

```
backends differ at operation 13 (Commit { claim: 2, epoch_delta: 0, end: Done, state: Unchanged, … })
  first:  "lease-lost"                                             (InMemoryStore)
  second: {"committed":{"version":0,"sent":[],"cancelled":[]}}     (SqliteStore)
```

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

One of the two stores is wrong about a commit, in a list with a lease that ran out, a paused actor and a create
inside a commit. If it is `SqliteStore`, a real node can commit a turn it should have lost (S4, S5, G1). If it is
`InMemoryStore`, tests on `TestActors` do not show what a real store does. Until it is fixed, any CI run can find
the same list again, and every release needs a green CI.

## What Changes

- Find which store answers this list wrongly, against S4 and S5 of `docs/architecture.md`. Check what
  `PostgresStore` answers too.
- Fix that store.
- Add the `cc` line above to `crates/cicada-fuzz/tests/backends.proptest-regressions`, so the list stays tested.
- If the architecture does not say which answer is right, say it there.

The simplest version: fix the one store, and add no new store behaviour.

To decide in the design:

- Which answer is right, and why.
- Whether the conformance suite gets a case for it, so that PostgreSQL is covered as well.

## Capabilities

### New Capabilities

None expected.

### Modified Capabilities

None expected: this fixes a store against the existing requirements. If the design finds that a requirement is
unclear, it names the spec then.

## Out of scope

- Other differences that the harness may find later; each is its own change.
- Changing how the comparison works, or how many lists a normal test run tries.
- Whether `InMemoryStore` stays at all; that is `private-store-trait`.

## Order

The first one: until it lands, any CI run can fail on this list. Before `every-actor-has-state`. It is not a step of
ADR 0018.

## Impact

- One of `crates/cicada-test` (`InMemoryStore`) and `crates/cicada-store-sqlite`, perhaps
  `crates/cicada-store-postgres` and `cicada-store-conformance`, and the regressions file of `cicada-fuzz`.
- `CHANGELOG.md`. No ADR unless a guarantee turns out to be wrong.
- Not breaking, unless the fix changes what a real store answers; the design says so then.
