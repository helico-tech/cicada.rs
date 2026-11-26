# Tasks

Normal lane, additive: in the 0.25.0 batch (check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Follow `design.md` and ADR 0074 (`docs/adr/0074-a-tray-crate-next-to-the-app.md`); the spec is
`specs/tray/spec.md` (a new capability). Crates: `cicada-tray` (new), `cicada-app`.

- [x] 1. **cicada-app.** `App::type_names()` (decision 5) and `App::on_worker_ready(f)` with
  `WorkerReady { grace_period, cancel_timeout }`, called once a worker over HTTP has started (decision 3).
  Verify: tests in `cicada-app`.
- [x] 2. **The crate and the state.** `crates/cicada-tray` in the workspace (portable), `Tray::new(factory)`,
  `Tray::with_env`, `connect`, `disconnect`, the `TrayState` watch with the transitions of decision 3. Verify: tests on
  virtual time against an in-process server.
- [x] 3. **Turns.** The tray's `TurnObserver`: running turns, a ring of the last 50 finished ones, `abandoned` through
  `Drop` without `turn_ended`, a redraw counter (decision 4). Verify: the turn tests of the design.
- [x] 4. **Changelog and docs handoff.** The 0.25.0 section names the crate, `type_names` and `on_worker_ready` under
  Added; the handoff gives the app guide "A menu bar worker". No edit to `README.md`, `AGENTS.md` or the handwritten
  docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate tray-worker-control --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches the `worker` command's code path; wait for the lock in short
  tries, the run as its own background command with a 2-hour timeout).
