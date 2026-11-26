# Tasks

Normal lane, version 0.26.0: 0.25.0 is released before this lands, and the `WorkerReady` change is **BREAKING**
(design decision 8; builder.md, Version). Builds after `app-node-id-after-types` (also changes `cicada-app`; rebase onto
it when it landed first). Follow `design.md`; the spec is `specs/menu-bar-tray/spec.md`. No ADR.

- [x] 1. **What the model needs from `cicada-app`.** `WorkerReady::types` (`#[non_exhaustive]`, no `Copy`), filled
  from the worker's picked types; `cicada_app::Contexts::load(&Env)` with `names()` and `current()`, no servers or
  tokens (decision 8). Verify: `cargo test -p cicada-app` with tests for both.
- [x] 2. **The menu model.** Module `menu`, every platform: `MenuInput`, `MenuModel` and `menu()`, with elapsed times
  clamped at zero and the badge; the quit and pending-pick state machine beside it (decisions 1, 3, 9). Verify: the
  unit tests of the design.
- [ ] 3. **The AppKit view.** Module `appkit` (`cfg(target_os = "macos")`): `Tray::run`, the runtime and its shutdown
  order, the macOS version check and subtitle fallback, the status item, the menu with the header view, the
  `define_class!` target with the 250 ms timer updating items in place, Quit, Ctrl-C and `applicationShouldTerminate:`;
  every `unsafe` with its reason; `objc2`, `objc2-foundation`, `objc2-app-kit` as macOS-only dependencies; elsewhere
  `run` returns an error (decisions 2–6, 10). Verify: `cargo build -p cicada-tray`, `cargo clippy -p cicada-tray
  --all-targets`, `cargo deny check advisories`, and the manual check list below on this Mac, each item noted as done
  or not done in the report.
- [x] 4. **Example, changelog and docs handoff.** `examples/menu-bar-worker.rs` (macOS only); the changelog's
  unreleased section names `Tray::run`, `WorkerReady::types` and `Contexts`; `docs-handoff.md` carries the design's
  Docs section. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples
  --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate tray-menu-bar --strict`, commit. The change loop runs the gate at
  landing.

Manual check list (task 3): the status item in a light and a dark menu bar, dimmed when offline; the menu by keyboard
and with VoiceOver; the running turns and times updating while the menu and a submenu are open; connect and disconnect
with the switch; a context pick while online; Quit while a turn runs; Ctrl-C from a terminal, also while the menu is
open; logout while online (or `osascript -e 'tell application "System Events" to log out'` if it is safe to try).
