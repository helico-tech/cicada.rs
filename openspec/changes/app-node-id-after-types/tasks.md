# Tasks

Normal lane, in the 0.25.0 batch (check the `release:` line of `python3 scripts/status.py`, builder.md, Version).
Builds after `tray-worker-control` (also changes `App`: `type_names`, `on_worker_ready`; rebase onto it when it landed
first). Follow `design.md`; the spec is `specs/app/spec.md`. No ADR.

- [x] 1. **Parse, then apply.** Split `settings::resolve` and `resolve_http` into a parse step (every flag and variable
  read and checked, with the types) and an apply step that lays the parsed values over a `NodeConfig`; behaviour
  unchanged (decision 4). Verify: the existing settings tests and `cargo test -p cicada-app`.
- [x] 2. **The built-in store check moves.** `check_built_in` is called from `lib.rs` on the parsed store, before the
  point where the hook will run, instead of inside `open_unless_stopped`; one path; a worker over HTTP skips it
  (decision 4). Verify: the existing test of a store whose feature is not built.
- [ ] 3. **`App::node_config_for`.** The method (synchronous, `FnOnce(&[&'static str]) -> anyhow::Result<NodeConfig>`),
  one slot with `node_config` (the last call wins), called once after every local check (usage errors, the HTTP
  refusal of `on_start`/`task`, `http_target`, the built-in store check) and before the wait for the store or server,
  with the run's types; flags override its fields; an `Err` fails with exit code 1; the worker over HTTP takes its id
  and the other fields it takes today. Rustdoc of the method, the module (the order of the settings) and `App`'s
  `Debug` updated (decisions 1–4). Verify: the design's tests.
- [ ] 4. **Changelog and docs handoff.** The 0.25.0 section names the method under Added; `docs-handoff.md` carries the
  design's Docs section with decision 5's notes. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `CICADA_UPDATE_CLI` only if the CLI help changed, `openspec validate
  app-node-id-after-types --strict`, commit, then `python3 scripts/gate.py run` (wait for the lock with short tries;
  the run as its own background command with a 2-hour timeout).
