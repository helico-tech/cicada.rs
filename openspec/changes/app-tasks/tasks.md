# Tasks

Normal lane, additive: in the unreleased batch (0.22.0, check the `release:` line of `python3 scripts/status.py`,
builder.md, Version). Follow `design.md` and ADR 0064 (`docs/adr/0064-background-tasks-of-an-app.md`); the spec is
`specs/app/spec.md`.

- [ ] 1. **The API.** `App::task(name, f)` and the public `Stopping` (`Clone`, `async fn wait`, `fn is_set`) in
  `crates/cicada-app/src/lib.rs` (decisions 1 and 6), with rustdoc of the start, the early-end rule and the stop.
  `serve` and `worker` refuse two tasks with one name (exit code 2); a worker over HTTP refuses an `App` with a task
  before it reads the config file, and its message and the `worker` help name tasks; regenerate `docs/reference/cli.md`
  (`CICADA_UPDATE_CLI=true cargo test -p cicada-app --features tui`). Verify: `cargo test -p cicada-app --all-features`.
- [ ] 2. **The life of a task.** In `crates/cicada-app/src/node.rs` (decisions 2 to 5): tasks start after `on_start`,
  before `serve` listens and in `worker`; one way to stop shared by every path once they run (a signal, an early end,
  a failed `Listening on`, a failed server): `Stopping` and the node's stop together, tasks waited for within the
  node's `shutdown_grace_period` and then aborted with a warning and not waited for, the store last. An early end
  (`Ok`, `Err`, panic) fails the command with exit code 1 naming the task; one recorded before a signal wins; an error
  after the stop is a warning with exit code 0. Verify: `cargo test -p cicada-app --all-features`.
- [ ] 3. **Tests, changelog and docs handoff.** The tests of the design's Tests section. The unreleased section of
  `CHANGELOG.md` names `App::task` and `Stopping` under Added. `docs-handoff.md` carries what the design's Docs
  section lists; nothing blocks unless a check says so. No edit to `README.md`, `AGENTS.md` or the handwritten docs.
  Verify: `cargo test -p cicada-examples --test docs`, `--doc` and `--test skill`.
- [ ] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate app-tasks --strict`, commit, then `python3 scripts/gate.py run --e2e`
  (the change touches `serve` and `worker`).
