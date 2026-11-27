# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-nodes` (its Running Turns and Nodes pages; rebase onto
it when it landed first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews are
`.reviews/inspector-soft-refresh.design.*`. No ADR.

- [ ] 1. **The re-read range and the seam.** A `soft_refresh` on the pager's lists: the range ending at the old last key
  (Running Turns, Watches) or at the page holding the old last id (Messages, Outcomes), at most 300 rows; new rows
  replacing the range; rows past it kept and marked, duplicates removed, Messages' and Outcomes' seen sets rebuilt; the
  surviving cursor; a range with no more dropping the rows past it (decisions 1 and 2). Verify: the range and seam
  tests of the design, including an old last message that was consumed (the read runs to 300).
- [ ] 2. **Generation, timing and bounds.** The list generation unchanged (its rustdoc updated), the soft refresh's own
  counter, waiting while a page or Retry is in flight, skipped while paging failed, one at a time with one remembered
  request, cancelled by Refresh, prefix, status, page and context; 4 requests at a time and the 10 s deadline (decisions
  3 and 7). Verify: those tests on virtual time.
- [ ] 3. **Selection, panes and cached pages.** The selection kept by id or cleared with an empty pane; panes from rows
  refilled, reading panes re-read without "Loading"; Watches replacing its loaded rows and filtering again; Types,
  Nodes and the Overview re-read with their selection, a failed soft refresh keeping the old list marked (decisions 4
  and 6). Verify: those tests against an in-process server.
- [ ] 4. **The scroll position.** Every `reloadData` of a page's table keeps the first visible row by key, or the same
  offset when it is gone; borrows released before AppKit calls (decision 5). Verify: `cargo build -p cicada-inspector`,
  `cargo clippy -p cicada-inspector --all-targets`, and the manual check list below, each item noted as done, run by
  code (not seen), or not done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names the scroll position kept across
  reloads under Changed; `docs-handoff.md` carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or
  the handwritten docs. Then `cargo fmt --all --check` (lines ≤120, comments included), `cargo clippy --workspace
  --all-targets`, `cargo test --workspace`, `cargo test -p cicada-examples --test docs` and `--doc`, `openspec validate
  inspector-soft-refresh --strict`, commit. The change loop runs the gate at landing.

Manual check list (task 4): on each paged page, scroll down, load more, and Refresh or page on: the first visible row
stays; a row deleted elsewhere disappears on the next reload without the view jumping; the footer's "not refreshed"
mark for rows past 300; keyboard and VoiceOver unaffected.
