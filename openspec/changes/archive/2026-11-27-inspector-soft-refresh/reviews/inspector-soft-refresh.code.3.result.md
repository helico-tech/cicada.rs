Verdict: fix needed

## Blocking

1. **A moved row at the end of the range, or many moved rows, still drop pending rows**: `crates/cicada-inspector/src/pager.rs:399-414`. The patience sorting itself is right: `tails.last()` ends a longest run, the latest one found. But a moved row can be in that run. Proved with a scratch test of `replace_range`:
   - Two actors, A (299 rows) then C (150). Deliver Now on C140 puts it first in C's block, at the last place of the 300-row range. The run a0..a298 + c140 has 300 rows, so the seam follows c140: c000..c139 (140 rows) vanish. With c140 second to last, the tie goes to c140 too (it comes later), and 139 vanish.
   - Bulk Deliver Now on m280..m439 of 450: the 160 moved rows form the longest run, and m140..m279 vanish.
   Both err toward dropping, not toward keeping. Fixed looks like: a moved row never pushes the seam past old rows the range does not hold. For example, keep every old row the range does not hold, marked "not refreshed". Add tests for the actor-block case and the bulk case.

## Answers

- Consumed in the middle, three moved ahead, new rows mixed in, moved back inside the range: no loss.
- Tests: the new test covers only a moved row at the front of the range.

## Notes

None.

## Not checked

I ran the `cicada-inspector` tests at head: 97 passed, 1 doctest passed. Not run: the workspace, clippy, fmt, the gate, AppKit.
