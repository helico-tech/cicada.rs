# Tasks

Normal lane, version 0.26.0 (unreleased). Follow `design.md`; the spec is `specs/menu-bar-tray/spec.md`; the design
reviews are `.reviews/tray-menu-mockup.design.*`. GUI change: match the boards in `mockup/` (Main, Disconnected,
Disconnecting, Running, Registered; `mockup/SOURCE.md`), with only the departures the design lists (all agreed). A
departure you find necessary is a question for the owner: stop and report it.

- [x] 1. **The app's two additions.** `cicada_app::Contexts::host(name)`: host and port of the context's server, parsed
  when loaded, never the URL, user, password, path or query; `WorkerReady::concurrency` from the started worker's
  config (decisions 2 and 6). Verify: their tests, with a server URL that holds a user, password, path and query.
- [ ] 2. **The tray's data and the menu model.** Crate-private: when stopping began, the grace period, whether Stop Now
  was used, when a connection failed, the failure's display text (credentials, queries and control characters removed,
  200 characters); the offline pick in `MenuControl`. The model: every state's sections, order, texts and counts as
  decisions 2–5 say, the recent rows per outcome, the bar's fraction and text, "no contexts" (decisions 2–5 and 8).
  Verify: the model tests on every platform, one per state and per recent outcome, the pick without connecting, the
  bar with zero grace and after Stop Now.
- [ ] 3. **Icons.** The tinted circle images per kind and appearance, made once and swapped on an appearance change;
  the status item's hexagon, filled when connected and hollow and dimmed otherwise; images shown where the system hides
  them by default (decisions 1 and 7). Verify: `cargo build -p cicada-tray`, `cargo clippy -p cicada-tray
  --all-targets`.
- [ ] 4. **The AppKit menu.** Items with images, subtitles (14.4) or joined titles (14.0–14.3), string badges; the
  header with the disconnecting bar; the not-connected, connected, connecting and disconnecting menus and the submenus
  as the boards show; identity by represented object; every `unsafe` with its reason, borrows released before AppKit
  calls (decisions 1–5). Verify: the build, clippy, and the manual check list below, each item noted as done, run by
  code (not seen), or not done.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names the new menu, `Contexts::host` and
  `WorkerReady::concurrency`; `docs-handoff.md` carries the design's Docs section; no edit to `README.md`, `AGENTS.md`
  or the handwritten docs. Then `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test
  --workspace`, `cargo test -p cicada-examples --test docs` and `--doc`, `openspec validate tray-menu-mockup --strict`,
  commit. The change loop runs the gate at landing.

Manual check list (task 4), for the owner: next to each board, light and dark: Main (connected), Running (Recent Turns
open), Registered (Registered Types open), Disconnected (offline, and after a failed connection), Disconnecting (with a
running turn, then Stop Now); the connecting menu; a context pick while offline selects without connecting; ⌘K
connects; VoiceOver reads the rows, the header and the bar.
