# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-actors` (same crate; rebase onto it when it landed
first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews are
`.reviews/inspector-children.design.*`. No ADR.

- [x] 1. **Identities and per-parent children in the model.** Row identities as paths, the selection as a path; each
  parent's first 100 children on opening and "more…" by its unchanged cursor, one request marker per parent, Retry and
  "No children.", closed rows keeping their children; reads carrying the list's generation and the parent's identity
  (decisions 1 and 2). Verify: the model tests of the design for children, "more…" twice and open, close, reopen.
- [x] 2. **What a child row shows, and fold state.** Address, type and running status ("—" outside the sample); the set
  of open identities kept across a reload by the rule of decision 4, forgotten once a page passed the row or its list
  ended (an open row under a closed parent is forgotten when that parent is read again), and reset for a different
  prefix or context (decisions 3 and 4). Verify: the fold-state tests of the design, including typing a letter and
  deleting it.
- [x] 3. **The outline.** `NSOutlineView` with the same columns, one retained object per identity removed only after
  `reloadData` replaced it, re-open via `expandItem` under the guard, `render_list` re-selecting by path and
  `wants_more` counting root rows, keyboard folding; every `unsafe` with its reason (decisions 1 and 5). Verify:
  `cargo build -p cicada-inspector`, `cargo clippy -p cicada-inspector --all-targets`, and the manual check list below,
  each item noted as done or not done in the report.
- [x] 4. **Changelog and docs handoff.** The 0.26.0 section names the folding; `docs-handoff.md` carries the design's
  Docs section. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples
  --test docs` and `--doc`.
- [x] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate inspector-children --strict`, commit. The change loop runs the gate at
  landing.

Manual check list (task 3): a parent row opens to its children and "more…" loads the next page; ← and → fold and unfold;
an actor shown at the root and as a child selects and folds apart; ⌘R keeps the open rows that still exist; a different
prefix folds everything; selecting a child fills the pane; scrolling still loads more root rows; keyboard and
VoiceOver; light and dark.
