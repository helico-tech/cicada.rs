Verdict: not yet

## Blocking

1. **Config and create (design, Decisions 2-3).** Host `PlacementSettings` breaks the pattern: permits and heartbeat keep config in the state, fixed at create. The example never creates the pool, though actors need an explicit create. ADR 0067 fixes 1000 nodes; the design makes it a setting. Fix: put `silence` (and `maxNodes`) in `PlacementState::new`, and say who creates the pool.
2. **Timer error paths (Decision 3).** Not named: the check message, refusing one the pool did not send, and recovery from a lost timer (heartbeat has `armed` and `Rearm`). A failing alert send, done like permits' `tell`, fails the whole check: nobody is marked silent and the timer is gone. Fix: name the timer; send each notice in its own self-sent turn (like `Notify`); re-arm; add scenarios.
3. **`Forget` vs `Gone` (Decision 4, placement-drain-and-gone).** Both remove a silent node. Once holders exist, `Forget` drops them without telling them. Fix: drop `Forget` here (the Why does not need it), or name it `Gone` now. Update ADR 0067 and its index row.

## Notes

- `NodeId` has no length limit: set 1-200 characters, no NUL.
- Labels "a set": duplicates refused or merged? Stored order?
- 1000 nodes × 32 × 100 characters is about 3 MB of state per offer, about 67 turns/s at 15 s. Lower the bounds or document it.
- "refused and logged": say "refused for good". The handoff should say: do not host the pool only on its members.

## Questions

- Q1: Is the pool kept out of releases until `placement-asks` lands? `status.py` can say "release now" in between.
- Q2: Does `placement-member`'s claim check need an instance field in `Offer`? If so, add it now.

## Not checked

Factory's `pool.rs`, ADR 0054, builds and tests.
