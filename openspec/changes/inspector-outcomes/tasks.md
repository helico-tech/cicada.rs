# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-types-watches` (same crate; rebase onto it when it
landed first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews are
`.reviews/inspector-outcomes.design.*`. No ADR.

- [x] 1. **The Outcomes list.** `outcomes.rs`: `outcomes` through the pager by status and prefix, the first row per id
  within a generation and paging past a page of only duplicates, the status switch resetting cursor, seen ids, search,
  selection and pane work together, the status column on All (decision 1). Verify: the list tests of the design.
- [x] 2. **The failed count.** The sidebar count from `/stats` with no prefix, read with the Overview's refresh and
  Outcomes' Refresh; "…", a kept value with the error after a failed read, "—", and reset on a context switch
  (decision 2). Verify: the count tests of the design.
- [x] 3. **The pane and the cause chain.** The pane from the row with the payload formatted on selection; Show Actor;
  Caused by and Caused as places with a fresh `status` read, an outcome's place keeping the outcome as read, one Caused
  request at a time, Retry on the same cursor, empty pages with a next cursor followed (several in a row too), "None",
  "not found", a failed linked read with Retry (also from a place reached by back or forward), the history cleared on a
  context switch (decisions 3 and 4). Verify: the pane and chain tests of the design.
- [x] 4. **The Outcomes page.** The `appkit` page: an `NSTableView` sharing the search field, footer and pane toggle,
  the status switch in the toolbar, the sidebar count, the pane's links; every `unsafe` with its reason, borrows
  released before AppKit calls (decision 5). Verify: `cargo build -p cicada-inspector`, `cargo clippy -p
  cicada-inspector --all-targets`, and the manual check list below, each item noted as done, run by code (not seen),
  or not done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names the Outcomes page, the cause chain
  and the sidebar count; `docs-handoff.md` carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or
  the handwritten docs. Then `cargo fmt --all --check` (lines ≤120, `Cargo.toml` and comments included), `cargo clippy
  --workspace --all-targets`, `cargo test --workspace`, `cargo test -p cicada-examples --test docs` and `--doc`,
  `openspec validate inspector-outcomes --strict`, commit. The change loop runs the gate at landing.

Manual check list (task 4): Outcomes lists failed outcomes by default and the switch shows Cancelled, Done and All with
its status column; the sidebar shows the failed count; selecting an outcome shows its error, envelope and payload (light
and dark); Caused by and Caused follow the chain, back and forward step through it; Show Actor; ⌘R; a stopped server;
keyboard and VoiceOver.
