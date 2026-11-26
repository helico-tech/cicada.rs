Verdict: pass

## Blocking findings

None. Tasks 1-5 are ticked and in the diff. Each spec scenario has a test in `crates/cicada-actors/tests/placement.rs` (fourteen new, named per scenario). Version 0.24.0 in `Cargo.toml`, both skills and `plugin.json`; CHANGELOG names asks; contract reference regenerated; gate has a full pass of 2278afed. `docs/guide/actors.md` equals 139b0857 (`git diff 139b0857 2278afed -- docs/guide/actors.md` is empty), holds no code, and describes the code at head (constructor, answers table, line skipping, Notify refused, release/cancel, shrink). The handoff is true; section 2 (architecture row) is honestly marked not written.

## Gap-fills judged

- `poolFull` vs `lineFull`: sound, and consistent across rustdoc, CHANGELOG, guide and `ask()` (Some+no room+line full = poolFull; None+line full = lineFull).
- One top-level `holders` list naming its node: fine. The pool is unreleased, the uniqueness and grant checks are simpler, and later proposals (placement-holders, drain-and-gone) assume nothing per node. Design decision 6 still says "holders per node"; worth a line when archiving.
- A foreign `Notify` refused for good: matches this pool's own `Check`/`Alert` rule and the design's "for the pool's own use". The guide states it.
- `maxWaiting`/`maxHolders` required at create: consistent with `silence`/`maxNodes`; no stored states exist yet.
- Dropped test: acceptable. A Notify queued by a release always runs before a repeated Ask sent later, so the order cannot be built with the helpers. The guard is one `!h.notified` condition, like the permit pool's, and no spec scenario needs it.

## Notes

1. `Notify { holder }` carries no grant, so a late failed Notify could free a later grant if a holder id were reused. The permit pool has the same gap, and the docs say to use a fresh id per attempt.
2. `MAX_BOOKS` names both limits; `MAX_ASKS_AND_HOLDERS` would read more plainly.

## Not checked

No suites run (I relied on the gate evidence). E2E was not run. I did not read the regenerated contract JSON line by line.
