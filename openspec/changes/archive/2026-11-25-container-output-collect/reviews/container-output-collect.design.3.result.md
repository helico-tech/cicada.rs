Verdict: not yet

Round 2: findings 1-4 resolved in design and specs (phase `Collecting` + `pending`, MODIFIED end and progress requirements with scenarios, `size` dropped, `outputs` always present with a store); round 2's notes resolved too. Two contradictions came with the fixes.

## Blocking

1. **ADR 0066 still describes the old design.** `docs/adr/0066-container-outputs-at-the-end.md`, Decision: "collects ... after it has recorded an end" (now: held pending in `Collecting`, recorded when the collection finishes) and "the first 10 skipped with path, size and reason" (size dropped). It does not name the state contract change (new phase, `pending`). The ADR belongs to this unbuilt change: align it with design decisions 1 and 4.
2. **"Recorded" now means two things; the errors requirement has two readings.** The MODIFIED end requirement says that with a store the end is recorded only in the commit that finishes the collection, and `end` stays empty in `Collecting`. The MODIFIED "A deadline and errors in a row" then says "a container without an end SHALL end `failed` with `runtimeErrors`". Read literally, that covers a `Collecting` container. It also says "while a recorded end's outputs are collected", a state the end requirement says does not exist. Design decision 5 ("`runtimeErrors` stays for a container without a recorded end") has the same problem. Fix: name the phases. `runtimeErrors` applies in `New`/`Started`; in `Collecting` the pending end is delivered with its outcome. Also add "(with a store, the turn that finishes the collection)" to the progress scenario "the turn that records the end".

## Notes

- `SkippedFile` (`copy.rs:46`) derives no `Serialize`/`Deserialize`/`JsonSchema`. Say that it gains them (additive), since `firstSkipped` puts it on the wire.
- `check()`: also say that `Collecting` requires `pending` and no `end`, not only that `pending` requires `Collecting`.
- With a store, does a `notStarted`, `missing` or `runtimeErrors` end carry `outputs` (decision 2: "always present") or not (decision 1: only ends the container ran for collect)? Say which.
- The new `Phase` variant means 0.22 cannot read a `Collecting` state. The upgrade handoff could warn against rolling back.

## Questions

None.

## Not checked

The container-progress worktree code, Docker behaviour, fixtures, actor code beyond `wire.rs` and `copy.rs`.
