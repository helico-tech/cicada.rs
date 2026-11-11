# Design

Described with the owner on 2026-11-11 (change loop, Describe). The owner chose a lighter loop for this change: no
design review, but a builder and a validator as usual (decision 3).

## Context

An agent reproduced the failing list on 2026-11-11 (`main` at deb1894) and traced it through both stores. **Neither
store is wrong. The comparison sends the two stores different commits.**

How the two runs part ways:

1. Operation 6 leaves two messages due at the same instant: `m18` on `alpha/1` (rescheduled) and `m1` on the
   re-created `alpha/0`.
2. Operation 8 claims both. Epochs come from one counter per store, and on a tie the stores hand them out in a
   different order. The in-memory store sorts by (due, address): `alpha/0` gets 2, `alpha/1` gets 3
   (`crates/cicada-test/src/memory.rs`, the claim). SQLite orders by `next_due_at` only, so a tie comes in row order,
   and the re-created `alpha/0` is behind `alpha/1`: `alpha/1` gets 2, `alpha/0` gets 3
   (`crates/cicada-store-sqlite/src/store.rs`, the claim query).
3. Operation 11 claims `alpha/0` again, with epoch 4 in both. `alpha/1` is paused.
4. Operation 12 commits `alpha/0` with `epoch_delta: -1`, so with the raw epoch 4 − 1 = 3. Both stores answer
   "lease lost", rightly. The harness then closes every open claim that matches (address, raw epoch)
   (`crates/cicada-fuzz/src/compare.rs`, `close_claim`). In SQLite that is the old claim of `alpha/0`, which has
   epoch 3 there; in memory nothing matches.
5. Operation 13, `claim: 2`, therefore picks different claims: in memory (`alpha/0`, epoch 2), stale, "lease lost";
   in SQLite (`alpha/0`, epoch 4), current, committed.

Without the delete and re-create both stores give the same order and the list passes.

Why no store is wrong: S4 promises the order of epochs per address only, S6 the order per instance only, and both
stores meet S5 for the request each of them got. The header of `compare.rs` already says that the order of claims
is free and that epochs are compared by rank per instance. `epoch_delta` breaks that rule in `Side::claimed`
(`claim.epoch + delta`): arithmetic on raw epochs only means the same in both stores when they hand out epochs in
the same order.

PostgreSQL was read, not run (Docker was down): it has the same fencing rule, one sequence for epochs, and
`ORDER BY next_due_at` without a tie-break.

The conformance suite misses nothing here. Its cases for the parts pass: a commit after the lease ran out, a stale
epoch after a re-claim, epochs across delete and re-create, and a paused actor.

The same cause shows with `Renew`, with `Release` and with `epoch_delta: 1`.

## Goals / Non-Goals

Goals:

- The same list reaches the same claims in both backends, whatever order they hand out epochs in.
- The list that failed stays tested.

Non-Goals:

- Any change to a store, or to what the architecture promises about the order of claims.
- Fewer stale epochs in the generated lists: `epoch_delta` keeps its three values and its weights.

## Decisions

### 1. The fix is in the harness: `epoch_delta` goes by rank per instance

Decided by the owner.

Each side of the comparison keeps, per address, the epochs it was handed in claims, in the order it got them. With
`epoch_delta: 0` an operation uses the claim's own epoch, as today. With `-1` it uses the epoch that the side saw
just before the claim's epoch for the same address; with `1` the one it saw just after. Where the side saw no such
epoch, it uses a number that it was never handed for that address: one below the first epoch seen for the address,
or one above the last. Such a number is not a lease of that instance in either store, so both answer alike.

`close_claim` keeps closing by the address and the epoch that was sent. With the rule above that epoch is the same
claim in both sides, or no claim in both.

This holds for `Commit`, `Renew` and `Release`, which all go through `Side::claimed`. The header of `compare.rs` gets
a line for it.

Not chosen: a tie-break by address in the claim queries of SQLite and PostgreSQL (store behaviour that the
architecture leaves free, added to production code for a test).

### 2. The order of checks in the PostgreSQL commit is left alone

Decided by the owner.

PostgreSQL checks "a turn cannot cancel its own message" before the lease; the other two stores check the lease
first. A stale commit that cancels its own message gets `Invalid` from PostgreSQL and "lease lost" from the others.
The runtime never sends such a commit (`Turn::cancel` refuses the turn's own message), and `private-store-trait`
takes the store trait out of the public API. Nothing is built for it.

### 3. A lighter loop, and a release of its own

Decided by the owner.

The change is a few lines in a test crate, and the investigation settled the facts. So there is no design review. A
builder builds it and a validator checks it, as for every change. It is released when it lands, as 0.3.2.

### 4. The change is named `fix-fuzz-claim-order`

Decided by the owner. It was proposed as `fix-backends-commit-difference`, on the premise that a store was wrong.

### 5. The failing list is kept twice

Decided by the main session.

The `cc` line of the proposal goes into `crates/cicada-fuzz/tests/backends.proptest-regressions`. A seed replays the
same list only while the strategy that builds lists stays as it is. So the fourteen operations are also written out
as a test of their own, which does not depend on the seed.

## Risks / Trade-offs

- **The floors of `generated_lists_reach_deep_states` may no longer hold**, because a `-1` or `1` now reaches other
  epochs than before. → The builder runs it and, when a floor must change, says why.
- **The harness may find another difference in a long run.** → It is not fixed here. The builder reports it with its
  `cc` line and does not put that line into the regressions file, because that would fail every CI run.

## Migration Plan

None. Nothing outside `crates/cicada-fuzz` changes.
