Verdict: not yet

Round 1: findings 1, 2, 4, 5 resolved; 3 half (see 3).

## Blocking

1. **Record-then-deliver contradicts the spec; its state is undesigned** (design 1). "The end is reported once, before removal" says the commit that records the end sends it. Today `Phase::Ended` with `end` means delivered: `step`/`work_left` stop, and `Start` delivers a saved `end` at once, so a `Start` mid-collection sends an end without `outputs`, then a second. Decide the state's mark (phase `collecting` or a field; the state schema is contract, `check()` ties `end` to `ended`/`removed`), MODIFY that requirement, add scenarios: `Start` and `Remove` mid-collection, a 0.22 `ended` state (no collection, no second end).
2. **Contradicts container-progress's spec**: "The last progress before the end" sends it "in the turn that records the end"; decision 1 moves it to the delivering turn. MODIFY it; say which turn reads the file.
3. **`size` has no source.** `SkippedFile` (copy.rs) has `path`, `reason` only; the size is in the text `"{size} bytes, more than the {left} left"` (not "over the byte limit" as the example). Add `size` to `SkippedFile` (non_exhaustive, additive; `Collector` fills it) or drop it.
4. **A log-only error has no home.** Store without `outputs()`: decision 2 leaves `outputs` absent, decision 5 puts the `max_errors` error in `outputs.error`. Say where it goes.

## Notes

- Still no spec scenarios for "no store" and "store `Unavailable`, then back" (Tests only).
- Decision 3's `Conflict` is the store's, decision 5's the runtime's: name the types.
- "Counts of the last attempt": say one turn does the copy and every `put_file`.

## Questions

- CO-C: `Remove` mid-collection waits for it (my pick, per decision 5) or cuts it short?

## Not checked

Docker `logs_since` on an exited container, fixtures, container-progress code beyond its design and spec.
