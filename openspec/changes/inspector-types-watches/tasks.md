# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-messages` (its pager and Show Actor; rebase onto it
when it landed first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews are
`.reviews/inspector-types-watches.design.*`. No ADR.

- [x] 1. **The Types model.** `types.rs`: one `types` read per refresh, first visit and next visit after a context
  switch; the local name filter; a failed read with Retry and no stale list; Refresh clearing the selection and the
  chosen schema; the pane's schema list, only the chosen schema formatted, a stale formatting dropped (decisions 1
  and 2). Verify: the Types tests of the design.
- [x] 2. **The Watches model.** `watches.rs`: paging through the pager, the local filter on watcher or target; with no
  filter, paging on scroll, and resuming it when the filter is cleared; with a filter, no page by itself, one page per
  Load more, Retry after a failed page also with no row visible; Refresh and a context switch starting again from the
  top and clearing the selection; Show Actor for the watcher and a one-actor target (decisions 3 and 4). Verify: the
  Watches tests of the design, including a filter that hides every row asking for no page, and both failures of the
  error scenario.
- [x] 3. **The two pages and the search field.** The `appkit` Types and Watches pages, `NSTableView`s sharing the
  search field, footer and pane toggle; the Types pane's schema pop-up; Load more in the footer; each page keeping its
  own filter text and placeholder; every `unsafe` with its reason, borrows released before AppKit calls (decisions 5
  and 6). Verify: `cargo build -p cicada-inspector`, `cargo clippy -p cicada-inspector --all-targets`, and the manual
  check list below, each item noted as done, run by code (not seen), or not done in the report.
- [x] 4. **Changelog and docs handoff.** The 0.26.0 section names the two pages; `docs-handoff.md` carries the design's
  Docs section. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples
  --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check` (lines ≤120, `Cargo.toml` and comments included), `cargo
  clippy --workspace --all-targets`, `cargo test --workspace`, `openspec validate inspector-types-watches --strict`,
  commit. The change loop runs the gate at landing.

Manual check list (task 3): Types lists the types and filters by name; selecting one shows its State schema, and the
pop-up switches to each message's schema (light and dark); Watches lists the watches, loads more on scroll without a
filter, and with a filter only on Load more; the filter matches watcher or target; Show Actor for a watcher and a
one-actor target, with back and forward; each page keeps its filter text when moving between pages; ⌘R; a stopped
server; keyboard and VoiceOver.
