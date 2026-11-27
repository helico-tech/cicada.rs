# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-soft-refresh-actors` (it schedules the soft refreshes
of both soft-refresh changes; rebase onto them when they landed first). Follow `design.md`; the spec is
`specs/inspector/spec.md`; the design reviews are `.reviews/inspector-live.design.*` (rounds 2 to 4 are of this design).

- [ ] 1. **The change source and the stream's lifecycle.** A private async trait over `ActorClient::changes`, a channel
  implementation for tests; one stream per page and prefix (local filters leave it alone), closed on page, prefix and
  context switches and on close; reconnect backoff (1, 2, 4, 8, 16, 30 s), reset 60 s after the first valid event
  (decisions 1, 2 and 7). Verify: the lifecycle tests on virtual time.
- [ ] 2. **Resync, the batch and the timer.** The first `resync` as the one trigger for open, reconnect and resume; the
  live model's batch (500 ms, 1,000 addresses, then everything), a soft refresh only when none runs, the prefix filter
  on paged pages; every soft refresh reconciling the pane; the 30 s timer re-armed after every refresh attempt, a
  skipped one too, and running while reconnecting (decisions 2–5). Verify: the batch and timer tests, including recovery
  after a Retry with no hints.
- [ ] 3. **Pause.** Pausing closes the stream, stops the timer, clears the batch and fences an automatic refresh in
  flight; resume's first `resync` runs one; ⌘R works in every state and re-arms the timer (decision 6). A refresh
  cancelled by pause or Retry marks the Actors page's root rows and the chosen parents whose reads it dropped "not
  refreshed" (a code review note of `inspector-soft-refresh-actors`). Verify: the pause tests, and the marks after a
  cancelled refresh.
- [ ] 4. **The Live pill.** The toolbar pill: Live from the first valid event, Reconnecting with its tooltip, Paused;
  clicking toggles; every `unsafe` with its reason, borrows released before AppKit calls. Verify: `cargo build -p
  cicada-inspector`, `cargo clippy -p cicada-inspector --all-targets`, the in-process server test of a real change
  starting the Actors page's soft refresh within a second of the hint, and the manual check list below, each item
  noted as done, run by code (not seen), or not done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names live updates; `docs-handoff.md`
  carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or the handwritten docs. Then `cargo fmt --all
  --check` (lines ≤120, comments included), `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `cargo
  test -p cicada-examples --test docs` and `--doc`, `openspec validate inspector-live --strict`, commit. The change
  loop runs the gate at landing.

Manual check list (task 4): with a node running, the Actors page follows a change within about a second and keeps the
selection, scroll and open rows; the pill shows Live, then Reconnecting when the server stops and Live again when it
comes back; Paused stops updates and ⌘R still reloads; resume reloads once; light and dark; VoiceOver reads the pill.
