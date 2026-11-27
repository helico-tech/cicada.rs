# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-outcomes` (same crate; rebase onto it when it landed
first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews are
`.reviews/inspector-nodes.design.*`. No ADR.

- [ ] 1. **Running Turns.** `turns.rs`: `turns` through the pager by prefix, the pane from the row, Show Actor; a
  Running Turns place keeping its prefix and the turn as read; back reading the actor's turn fresh by its exact address
  (`turn(address)`), not from cached or partial pages, to tell a turn now running from "no longer running" (decisions
  1 and 4). Verify: the Running Turns tests of the design, including paging while leases change and back to an ended
  or moved turn.
- [ ] 2. **Nodes.** `nodes.rs`: the cache read on the first visit, its Refresh and a context switch (at once when
  visible, else at the next visit), never on the Overview's refresh; the local id filter; "as of the last read"; a
  failed read with Retry and no stale list; every read with the shared read deadline (decision 2). Verify: the Nodes
  tests of the design, including a late list result after a context switch.
- [ ] 3. **A node's pane.** `node(id)` under the pane's generation, dropped on a new selection, Refresh or context
  switch, Retry on the selected node, "not found", its turns with Show Actor; a Nodes place reading its pane fresh on
  back (decisions 3 and 4). Verify: the node pane tests of the design, including A, then B, then A, and a selection
  the filter hides.
- [ ] 4. **The two pages.** The `appkit` Running Turns and Nodes pages, `NSTableView`s sharing the search field (each
  keeping its own filter text and placeholder), footer and pane toggle; every `unsafe` with its reason, borrows
  released before AppKit calls (decision 5). Verify: `cargo build -p cicada-inspector`, `cargo clippy -p
  cicada-inspector --all-targets`, and the manual check list below, each item noted as done, run by code (not seen), or
  not done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names the two pages; `docs-handoff.md`
  carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or the handwritten docs. Then `cargo fmt
  --all --check` (lines ≤120, `Cargo.toml` and comments included), `cargo clippy --workspace --all-targets`, `cargo
  test --workspace`, `cargo test -p cicada-examples --test docs` and `--doc`, `openspec validate inspector-nodes
  --strict`, commit. The change loop runs the gate at landing.

Manual check list (task 4): Running Turns lists the running turns and loads more on scroll; Show Actor from a turn, and
back after the turn ended shows "no longer running"; Nodes lists the nodes with their status and last heartbeat as the
server gave them; selecting a node lists its turns; a gone node says "not found"; ⌘R on each page; a stopped server;
keyboard and VoiceOver; light and dark.
