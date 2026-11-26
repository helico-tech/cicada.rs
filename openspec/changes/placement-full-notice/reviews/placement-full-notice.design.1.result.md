Verdict: not yet

## Blocking

1. **Full pool vs held key: which wins?** (design.md, Decision 2.) `offer` checks the contest before `maxNodes`, so today a new node offering a held key in a full pool is recorded in `contested` and gets a `conflict` alert. Decision 2 says an ignored offer records "no contest", which reads as the full check now coming first. That would change ADR 0072's scenario for a full pool. My pick: the contest stays first. Say so and test it.
2. **The mark outlives the full period.** (Decision 1, Tests.) After `Configure` raises `maxNodes`, or `Gone` frees a slot, the mark stays. If the pool fills again within the silence time, the next new node is ignored with no alert. Its offers keep moving `at`, so no alert ever comes. My pick: clear `full` when an offer adds a new node. Add a scenario.
3. **The proposal contradicts the design.** The proposal says one notice "per new node"; the design picks one per pool.

## Notes

1. Name the armed-check edge in a test: a full pool whose nodes are all silent, with no contest. The ignored offer must arm the check (today `offer` arms only on the add path), and the check must end the mark and clear `armed`.
2. `placement-settings` Decision 3 re-arms at "a live node's silence or a contest" and checks the silence bound "for some node". Say that this change extends both to `full` (use `next_check()` and `check()`). Also update the docs of `Rearm`, `Check` and `armed`.
3. The handoff must replace guide line 759 ("an offer from a new node … is refused") and the `PlacementActor` rustdoc. ADR 0075 needs its row in `docs/adr/README.md`.
4. In a rolling upgrade, an alert target on an older node fails the `full` alert for good. Name this in the changelog.

## Questions for the owner

- Q1: Mark `PlacementNoticeKind` `#[non_exhaustive]` now, inside the same breaking 0.25.0 line? My pick: yes.

## Not checked

PlacementMember logging, the contract snapshot files, and the E2E.
