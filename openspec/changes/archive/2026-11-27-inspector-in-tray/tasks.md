# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `inspector-window` (same crate; rebase onto it when it landed
first). Follow `design.md`; the spec is `specs/inspector/spec.md`; the design reviews are
`.reviews/inspector-in-tray.design.*`. No ADR (ADR 0076 already allows the edge).

- [x] 1. **The shared teardown and `Embedded`.** One teardown for the embedded and standalone window (timer, generation,
  aborted refreshes on the given handle); `Embedded::new`, `show`, `close` and its drop; the lifecycle as a plain state
  machine (decision 1). Verify: the lifecycle tests; `cargo test -p cicada-inspector`.
- [x] 2. **Focus and keys.** `show` activates and makes the window key and front; ⌘R, ⌘W, ⌘[ and ⌘] through the
  window's own key handling (decision 2). Verify: `cargo build -p cicada-inspector`, `cargo clippy -p cicada-inspector
  --all-targets`.
- [x] 3. **The tray's feature.** `inspector = ["dep:cicada-inspector"]`, the menu item (⌘I) and its disabled state while
  quitting, one `Embedded` in the view, the context passed, every quit path closing it first (decisions 3 and 4).
  Verify: the tray tests of the design with the feature on and off; the manual check list below, each item noted as
  done or not done in the report. Feature-on and feature-off tests passed on macOS. The manual GUI checks are pending.
- [x] 4. **The gate and the docs.** `scripts/ci/feature-builds.sh` runs check, clippy and test of `cicada-tray` with
  `--features inspector`; the dependency check passes with the feature on and off; the 0.26.0 section names the feature
  and `Embedded`; `docs-handoff.md` carries the design's Docs section. No edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Verify: `bash scripts/ci/feature-builds.sh`, the script tests if they cover it, `cargo test -p
  cicada-examples --test docs` and `--doc`.
- [x] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate inspector-in-tray --strict`, commit. The change loop runs the gate at
  landing.

Manual check list (task 3): Open Inspector… opens the window in front, on the connected context; a second pick brings it
to the front without changing its context; with Settings also open, ⌘W and ⌘R reach the key window; ⌘I works while the
menu is open; Quit, Ctrl-C and logout with the inspector open and a refresh pending all end cleanly; closing it with ⌘W
and with the close button, then reopening it, does not crash (code review); light and dark.

Notes from the second design review: set the quit flag before closing the inspector, also on an immediate AppKit
termination, and release mutable borrows before calling AppKit; test with the toolbar's Refresh in the overflow menu,
in the tray's Connecting and Stopping states, with a server-only target (`CICADA_SERVER`) and a malformed config; the
dependency check requires the edge with the feature and its absence by default.
