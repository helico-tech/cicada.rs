Verdict: fix needed

## Blocking findings

1. `crates/cicada-inspector/src/actors.rs:881` (`run`). At the start, every cached children page is marked `FirstPage`, including the parents about to be re-read. For an open parent with a complete page, `child_status_row` (`appkit/actors.rs:138`) adds a "not refreshed" row. The row goes away when the re-read lands, so the outline grows and shrinks on every refresh, and a parent being re-read is labelled "not refreshed" for up to 10 s. Scratch test: open `a` with children `[x]`, hold its re-read, and `children[a].not_refreshed()` is true. Fixed: mark only what this run will not re-read, and mark failures when they land.

2. `actors.rs:1236` (`land_sample`). A failed soft sample still moves `sample_applied` forward. If the first sample is still in flight and the soft sample fails, the first sample's late answer is dropped. `running` then stays `Loading` until the next refresh (scratch test). A failed read is not a newer status (decision 5). Fixed: move `sample_applied` forward only on success.

3. Spec scenario "An open row's child is deleted": no test checks that a selection on a surviving row stays. Both deletion tests select under the deleted path. The code works (scratch test). Fixed: add that test.

## Notes

- The last commit edits the agreed spec ("Many open rows" reworded, one scenario added), and the report does not say so.
- `retry_children` also drops a remembered refresh that has not started yet. The design does not say whether it should.
- The AppKit `unsafe` blocks have their reasons, and the borrows are released before AppKit calls.
- The handoff fits ADR 0045.

## Not checked

The GUI, the outline restore at runtime, VoiceOver, the gate and the workspace tests. I ran `cargo test -p cicada-inspector` (113 passed) and three scratch tests.
