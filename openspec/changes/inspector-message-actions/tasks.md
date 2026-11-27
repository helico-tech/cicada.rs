# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-send-message` and `inspector-actions` (the `actions`
module's alert text, one action at a time and the refresh request; rebase onto them when they landed first). Follow
`design.md`; the spec is `specs/inspector/spec.md`; the design reviews are `.reviews/inspector-message-actions.design.*`
(round 5 passed; its notes are folded into decision 7).

- [ ] 1. **Selection and the buttons by state.** A set of selected ids in the Messages and Outcomes models beside the
  pane's message, kept by id across a soft refresh, cleared by a manual Refresh, a new prefix, status or context; back
  and forward record only the clicked row; the panes' buttons by what they know (Messages: Cancel and Deliver Now on
  every row; Outcomes and followed links by state) (decisions 1 and 4). Verify: the selection and button tests.
- [ ] 2. **Single actions and the pane hold.** Retry with its sheet and the retries still kept ("at least N", "not
  read"); Cancel with its sheet and four outcomes; Deliver Now; the hold from just before the request, keyed by message
  and pane generation, deferring a soft refresh's pane change; the `status` read after the answer filling the pane and
  ending the hold, a terminal state kept, not found as gone, a failure applying the deferred change; what ends a hold;
  Caused re-read from its first page after a Retry under its own counter (decisions 2, 3, 6 and 7). Verify: the action,
  mapping and hold tests on virtual time, including a refresh landing before the write's answer on both panes.
- [ ] 3. **Bulk runs.** Chunks of 100 one after another with progress and Stop; bulk Cancel; bulk Retry's Caused
  checks (8 at a time, 60 s, a child or an empty page with a next cursor as retried, not found as gone, a failed read as
  not read), its sheet and the K = 0 case; the counts against the confirmed ids with "not eligible by then" and "not
  sent"; a failed chunk stopping the run, never sent again; a refresh after any run that may have written; a bulk sheet
  confirmed after a Refresh sending nothing (decision 5). Verify: the bulk tests.
- [ ] 4. **The AppKit tables, buttons, sheets and alerts.** Multiple selection in both tables; the pane buttons and the
  footer's bulk buttons with the 1,000 limit; sheets with Cancel as the default button, Return and Escape cancelling,
  Retry and destructive buttons only by click; alerts; every `unsafe` with its reason, borrows released before AppKit
  calls (decision 8). Verify: `cargo build -p cicada-inspector`, `cargo clippy -p cicada-inspector --all-targets`, and
  the manual check list below, each item noted as done, run by code (not seen), or not done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names the message actions;
  `docs-handoff.md` carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or the handwritten docs.
  Then `cargo fmt --all --check` (lines ≤120, comments included), `cargo clippy --workspace --all-targets`, `cargo test
  --workspace`, `cargo test -p cicada-examples --test docs` and `--doc`, `openspec validate inspector-message-actions
  --strict`, commit. The change loop runs the gate at landing.

Manual check list (task 4): with a node running, ⇧- and ⌘-click select several rows in Messages and Outcomes; Retry on
a failed outcome asks with its count and shows the new id; Cancel and Deliver Now on a pending message; bulk Retry
skips a retried row and shows its counts; bulk Cancel with progress and Stop; Return and Escape cancel every sheet;
light and dark; VoiceOver reads the buttons, sheets and the footer.
