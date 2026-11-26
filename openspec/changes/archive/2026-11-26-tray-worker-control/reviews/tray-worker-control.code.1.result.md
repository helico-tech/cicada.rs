Verdict: fix needed

Reviewer: claude-reviewer (other family than the builder, gpt-6.2-sol).

## Blocking

1. `openspec/changes/tray-worker-control/docs-handoff.md`: the handoff is incomplete. It names only the app guide
   section and the install pins, but a new crate also belongs in every place that lists the crates. Missing: the crate
   table in `docs/architecture.md` (around line 435), the Map table in `AGENTS.md`, the crate table in `README.md`
   (line 46 style row), a row in `docs/how-to/install.md` ("Run a menu bar worker"), and the crate list and a route
   in `skills/cicada/SKILL.md` (description and routing table). To see it: `grep -n cicada-containers` in those files
   finds a row, while the handoff names none of them for `cicada-tray`. Fixed: the handoff lists each place with the
   text to add, or says why one is left out.

## Notes

- Task 5 (finishing checks, the `--e2e` gate run) is still unticked and no gate evidence exists for e99b117d. The
  gate must run before landing.
- `Tray::connect`: when the spawned task panics or its runtime shuts down before `run_with` returns,
  `Control::finish` never runs. The connection stays `Some` and the state stays `Connecting`/`Online`, so every later
  `connect` does nothing. A drop guard in the spawned task that calls `finish(Failed)` would close this.
- `tasks.md` task 1 has a broken line wrap inside `WorkerReady { grace_period, cancel_timeout }`.
- `scripts/ci/dependency-trees.sh` does not check that no library crate builds `cicada-tray` (ADR 0074 says no library
  crate depends on it). Nice to have, like the `cicada-containers` check.

Checked and fine: Online comes only from `on_worker_ready`; a ready call after an early disconnect stays Stopping with
the settled `until`; `--context=<name>` forces HTTP mode and stops a context like `--help` being read as a flag; the
observer's `Drop`/`turn_ended` path cannot double-finish; the ring evicts at 50; `cicada-app` changes are additive and
documented; every spec scenario has a test; CHANGELOG and ADR row match; no protected docs edited; fmt clean.

## Not checked

Ran `cargo test -p cicada-tray` (8 pass) and `cargo fmt --all --check` only. Clippy, the workspace tests, the
`cicada-app` tests and the gate were not run.
