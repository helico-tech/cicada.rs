# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-children` (it changes the Actors list first; rebase
onto it when it landed first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews are
`.reviews/inspector-messages.design.*` (round 1 under the old name `inspector-tables`). No ADR.

- [x] 1. **The root pager.** `pager.rs`: the Actors list's root paging factored out, with its generation, debounce,
  Retry and cancellation; Actors (with its children) on top of it (decision 1). Verify: every Actors test passes
  unchanged.
- [ ] 2. **The Messages model.** `messages.rs`: `list_pending` under the prefix through the pager, the first row per id
  within a generation, the cursor advanced and paging rechecked after a page of only duplicates, even when the visible
  row count stays the same; the pane from the row, its payload formatted on selection only, the raw payload shared
  across view snapshots and a formatting result guarded by the selection (decisions 2 and 3). Verify: the Messages
  tests of the design.
- [ ] 3. **Show Actor.** The jump bound to its context and generation, recorded in back and forward and replayed on
  Forward, cancelled on leaving the page or a new search, its target kept through Retry; the exact root row selected
  after a successful load, "not found" and a failed read apart (decision 4). Verify: the Show Actor tests, including
  Back restoring the source prefix and the message from a later page.
- [ ] 4. **The Messages page.** The `appkit` page: an `NSTableView` sharing the search field, footer and pane toggle,
  the pane with Show Actor; every `unsafe` with its reason (decision 5). Verify: `cargo build -p cicada-inspector`,
  `cargo clippy -p cicada-inspector --all-targets`, and the manual check list below, each item noted as done or not
  done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names the Messages page and Show Actor;
  `docs-handoff.md` carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or the handwritten docs.
  Then `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `cargo test -p
  cicada-examples --test docs` and `--doc`, `openspec validate inspector-messages --strict`, commit. The change loop
  runs the gate at landing.

Manual check list (task 4): Messages lists the pending messages under the prefix and loads more while scrolling;
selecting one shows its envelope and payload (light and dark); Show Actor selects its actor on Actors, and back returns
to the message, forward to the actor; a gone actor says "not found"; ⌘R reloads; a stopped server shows its errors;
keyboard and VoiceOver.
