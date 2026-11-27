# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `stats-failed-count` and `tray-settings` (shares `cicada-app`;
rebase onto them when they landed first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews
are `.reviews/inspector-window.design.*`.

- [x] 1. **ADR 0076 and `context_client`.** The ADR (scope of decision 1) with its row in `docs/adr/README.md` and ADR
  0011's status line; the gate's dependency check gains `cicada-inspector` with the one named exception;
  `cicada_app::context_client` on `resolve` and `Target::client`, returning the effective context (decisions 1 and 3).
  Verify: its tests, including credential redaction for a transport error and a malformed server URL (the client's
  parse error quotes the raw base URL today); `cargo test -p cicada-app`; the script tests if the dependency check
  changed.
- [x] 2. **The crate and its model.** `crates/cicada-inspector` in the workspace; `Overview` with per-part results, the
  bounded refresh (8 at a time, 50 types, 10 s; completed results kept at the deadline; "not counted" apart from
  failed), a superseded refresh cancelled, the `/turns` first page; `Navigation` (decision 2). Verify: the model tests
  of the design against an in-process server.
- [x] 3. **The window.** The `appkit` module: `Inspector::run`, the runtime, the macOS check, the split view with the
  sidebar, the toolbar with back, forward, the context pop-up (showing when `CICADA_SERVER` or `CICADA_TOKEN` override
  the picked context) and Refresh, the Overview tiles, "Coming soon" for the other items, the window still usable after
  a target error; Close and Quit both invalidate the timer and drop pending refreshes; every `unsafe` with its reason
  (decisions 4 and 5). Verify: `cargo build -p cicada-inspector`, `cargo clippy -p cicada-inspector --all-targets`,
  and the manual check list below, each item noted as done or not done in the report. Built and reported: a start
  on this Mac against an in-process server stayed up for 5 s with nothing on stderr; screen capture was refused, so no
  item of the list was seen on screen. The owner runs it before 0.26.0 is released.
- [ ] 4. **Changelog and docs handoff.** The 0.26.0 section names the crate and `context_client`; `docs-handoff.md`
  carries the design's Docs section. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test
  -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `cargo deny check advisories`, `openspec validate inspector-window --strict`, commit. The
  change loop runs the gate at landing.

Manual check list (task 3): the window opens on the current context; the sidebar with vibrancy, light and dark; the
Overview tiles against `cicada stats`, `cicada get nodes` and `cicada get turns`; ⌘R refreshes; back and forward follow
the sidebar; the context pop-up switches and refreshes, and shows an environment override; a stopped server shows its
error on the tiles and the window stays usable; Close and Quit both end cleanly; keyboard and VoiceOver.
