# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-soft-refresh` (its machinery; rebase onto it when it
landed first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews are
`.reviews/inspector-soft-refresh-actors.design.*`. No ADR.

- [ ] 1. **Root rows and children by address.** The root range by the old last address with the sorted unique seam;
  every children page, re-read or from "more…", merged by address with the cursor rule; a gone child pruned with its
  caches, open state, rows and reads in flight before the selection is checked (decisions 1 and 2). Verify: the merge
  and pruning tests, including "more…" that exhausts the cursor dropping kept children absent from the newly covered
  range.
- [ ] 2. **Which open rows, in which order.** The view's report of open paths in screen order and the first visible
  row; at most 20 parents, re-read depth by depth with the visible-first ranking within a depth, each path checked
  against its parent's new children first; marks shown and cleared (decision 3). Verify: the ranking, depth and mark
  tests.
- [ ] 3. **Coordination and one budget.** Waiting for a root page or parent read in flight, one remembered request, a
  "more…" during it waiting, a Retry cancelling it, a failed parent not re-read and not counted, a failed or late
  re-read keeping its children marked; the turns sample with its sequence number; the shared 4-at-a-time limit and 10 s
  deadline across root, turns, children and pane (decisions 4 and 5). Verify: those tests on virtual time.
- [ ] 4. **The pane and the outline.** The pane re-read without "Loading", no doubled pane read, a failed part keeping
  its content beside its error, "not found" as today; the outline's report of open paths and first visible row, the
  restore after applying; every `unsafe` with its reason, borrows released before AppKit calls (decision 6). Verify:
  the pane tests, `cargo build -p cicada-inspector`, `cargo clippy -p cicada-inspector --all-targets`, and the manual
  check list below, each item noted as done, run by code (not seen), or not done in the report.
- [ ] 5. **Docs handoff and finishing checks.** `docs-handoff.md` carries the design's Docs section (the cost of full
  states named); no changelog entry unless a user-visible effect is found; no edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Then `cargo fmt --all --check` (lines ≤120, comments included), `cargo clippy --workspace
  --all-targets`, `cargo test --workspace`, `cargo test -p cicada-examples --test docs` and `--doc`, `openspec validate
  inspector-soft-refresh-actors --strict`, commit. The change loop runs the gate at landing.

Manual check list (task 4): with rows open at two depths and the view scrolled, a soft refresh keeps the open rows, the
selection and the first visible row; a child deleted elsewhere disappears with its open descendants; the pane keeps its
content without "Loading"; a "not refreshed" mark shows on a parent past the 20; keyboard and VoiceOver unaffected.
