# Tasks

Normal lane, version 0.26.0 (unreleased). Builds on `inspector-in-tray` (landed). Follow `design.md`; the spec is
`specs/inspector/spec.md`; the design reviews are `.reviews/inspector-actors.design.*`. No ADR.

- [x] 1. **The list model.** `ActorsList`: pages by the unchanged cursor, requests keyed by generation and cursor, at
  most one in flight, a failed page kept for an explicit Retry; the turns sample under the list's generation with its
  "first 200" and "unknown" footers; the list generation bumped at the first key press, the debounced request, and
  queued or pending work cancelled on a context switch or close (decisions 1–3). Verify: the list tests of the design,
  with delayed HTTP answers.
- [x] 2. **The pane model.** The pane's generation and reads: `get_actor`, `turn`, the mailbox's first 50 rows (the
  turn's message matched, the pending rows listed), the watches as watcher and as exact target kept apart; "not found",
  "idle", paused with running, one part failing alone (decision 4). Verify: the pane tests of the design, including A,
  then B, then A with delayed answers.
- [x] 3. **The JSON formatter.** Pretty JSON cut at 256 KiB on a UTF-8 boundary with a notice, tokenized into at most
  20,000 runs (decision 5). Verify: its tests on nested JSON, escapes, numbers, non-ASCII text at the cut and the run
  cap.
- [x] 4. **The AppKit table and pane.** The `NSTableView` with its columns and paging on scroll, the search field, the
  pane as an `NSTabView` with the two watch lists apart, the runs applied as UTF-16 ranges in one batch, the pane
  toggle and Refresh; every `unsafe` with its reason (decisions 5–7). Verify: `cargo build -p cicada-inspector`,
  `cargo clippy -p cicada-inspector --all-targets`, and the manual check list below, each item noted as done or not
  done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names the Actors page and the pane;
  `docs-handoff.md` carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or the handwritten docs.
  Then `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `cargo test -p
  cicada-examples --test docs` and `--doc`, `openspec validate inspector-actors --strict`, commit. The change loop runs
  the gate at landing.

Manual check list (task 4): Actors lists the actors under the search prefix and loads more while scrolling; typing a
prefix narrows it without stale rows; the status column marks running and paused actors; selecting an actor fills the
pane (state as highlighted JSON in light and dark, the running turn, Mailbox and both Watches lists); a large state
shows the cut notice and stays responsive; the pane toggle hides and shows it; ⌘R reloads; a stopped server shows its
errors and the window stays usable; keyboard and VoiceOver.
