# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `worker-stop-now` (same crates; rebase onto it when it landed
first). Follow `design.md` (eight decisions, five design review rounds in `.reviews/tray-settings.design.*`); the spec
is `specs/menu-bar-tray/spec.md`. No ADR.

- [x] 1. **`ContextFile` in `cicada-app`.** The accessors, `set_context` and `set_server_and_token`, `use_context`,
  `save`, and `update` with the lock on the resolved target; sanitized parse errors in the shared loader; `Debug`
  redaction of tokens, header values and URL passwords in `ContextEntry`, `Config` and `Context`; the shared URL check;
  the `config` operator commands on it, their output unchanged (decisions 1, 2, 8). Verify: its tests (two threads'
  updates both kept, a parse error without the token, `Debug`) and `cargo test -p cicada-app`; the CLI reference
  unchanged.
- [ ] 2. **`tray.toml` and the worker's arguments.** Location rules (relative, symlink, a config named or resolving to
  `tray.toml` or `tray.toml.lock` refused), its own lock, read and write; `connect` passes `--types` and
  `--concurrency` for a saved entry, `--types all` for every registered type, and fails for no registered type left or
  a lone literal `all`/`none` among others (decisions 3 and 4). Verify: the tray-file and argument tests of the design.
- [ ] 3. **`SettingsForm` and `apply`.** Load, validate (URL, types, concurrency 1–1,000, the name rule), revert,
  apply under both locks in order with only edited fields, the partial-save report and retry; the control's `apply`
  action (decisions 5 and 6). Verify: the form and `apply` tests of the design.
- [ ] 4. **The window.** The `appkit` window of decision 7 and the enabled Settings… item; every `unsafe` with its
  reason. Verify: `cargo build -p cicada-tray`, `cargo clippy -p cicada-tray --all-targets`, and the manual check list
  below, each item noted as done or not done in the report.
- [ ] 5. **Changelog, docs handoff and finishing checks.** The 0.26.0 section names `ContextFile`, the window and
  `tray.toml`; `docs-handoff.md` carries the design's Docs section; no edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Then `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`,
  `cargo test -p cicada-examples --test docs` and `--doc`, `openspec validate tray-settings --strict`, commit. The
  change loop runs the gate at landing.

Manual check list (task 4): Settings… and ⌘, open the window, bring it to the front and focus it; ⌘W and Esc close it
and it reopens with the values on disk; the context pop-up switches the fields; Add… creates a context and its sheet
can be cancelled; the token is masked; a bad server shows its error; Revert restores; Apply while online reconnects with
the new types (the Registered Types checkmarks follow); keyboard and VoiceOver; light and dark.
