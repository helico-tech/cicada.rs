# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-live` (the soft refresh of the Actors page, the Live
pause; rebase onto them when they landed first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design
reviews are `.reviews/inspector-actions.design.*`.

- [x] 1. **The `actions` module: calls, targets and checks.** Pause, Resume, Cancel Turn, Purge Mailbox and Delete
  through `cicada-client`; a target as the selected address with the pane's generation; one action at a time from the
  press (a sheet opening) to the answer, also across a reselection; the target checked before sending and on confirming
  ("selection changed; nothing done"); a context switch or closed window dropping a pending result (decisions 1 and
  2). Verify: the in-process server tests of each call, one action at a time and a changed target sending nothing.
- [ ] 2. **Sheets, results and alerts.** The sheet texts with their counts (pending rows among the mailbox's first 50
  rows without the running one, "at least N" from the cursor, "not read"; children loaded, "at least N", "children not
  read"); Cancel Turn's sheet and its true and false answers; 404s mapped to local text; any other error as an alert
  with the problem's title and detail or "HTTP <status>", control characters and credentials removed, each part cut at
  500 characters; the success notices (decisions 3–5). Verify: the text, mapping and alert tests.
- [ ] 3. **The fresh read after a write.** A soft refresh queued behind one running and never merged, also with Live
  paused; the paging fallback reading the pane only after the answer; Delete pruning every cached occurrence of the
  actor and its known descendants at once and fencing root and children pages read before the answer; a Delete that
  answers after another actor was selected leaving that pane and showing no notice (decision 6). Verify: the refresh,
  prune and fence tests on virtual time.
- [ ] 4. **The AppKit buttons, sheets and alerts.** The actor pane's buttons by state; sheets with Cancel as the
  default button, Return and Escape cancelling, the destructive button with no key equivalent; alerts; every `unsafe`
  with its reason, borrows released before AppKit calls (decision 7). Verify: `cargo build -p cicada-inspector`, `cargo
  clippy -p cicada-inspector --all-targets`, and the manual check list below, each item noted as done, run by code
  (not seen), or not done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names the actor actions;
  `docs-handoff.md` carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or the handwritten docs.
  Then `cargo fmt --all --check` (lines ≤120, comments included), `cargo clippy --workspace --all-targets`, `cargo test
  --workspace`, `cargo test -p cicada-examples --test docs` and `--doc`, `openspec validate inspector-actions
  --strict`, commit. The change loop runs the gate at landing.

Manual check list (task 4): with a node running, pause and resume an actor and see the flag change; Cancel Turn on a
long turn; Purge Mailbox and Delete each ask first with their counts, Return and Escape cancel, the destructive button
only by click; a refusal shows an alert and leaves the pane; light and dark; VoiceOver reads the buttons and sheets.
