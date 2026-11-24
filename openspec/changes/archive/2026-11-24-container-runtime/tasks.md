# Tasks

Patch 0.20.7, normal lane. A new crate only: no change to existing crates, guarantees, wire formats, schema or the
command line. Follow `design.md`; the spec is `specs/container-runtime/spec.md`.

- [x] 1. **The crate and its types.** Add `crates/cicada-containers` to the workspace (version from the workspace,
  dependencies `async-trait` and `thiserror` only, feature `testing`, `missing_docs` like the other crates). Write
  `ContainerSpec` (decision 7, `Debug` without secret values), `ContainerStatus` (decision 8), `ContainerError` and
  `ErrorKind` (decision 5), `Label`, `Mount`, `Networking`, `NetworkSpec`, `VolumeSpec`, `CopyFile`,
  `ContainerSummary` and `Swept`, all `#[non_exhaustive]` with a `new` where a caller builds one, and the
  `ContainerRuntime` trait with the doc of each method's repeat rule (decisions 6, 9, 10, 11; `create` may pull).
  Verify: unit tests for the scenarios "Debug output of a spec with a secret", "A new spec is plain" and
  `is_retryable`; `cargo test -p cicada-containers`; `cargo clippy -p cicada-containers --all-targets`.
- [x] 2. **The fake.** `FakeRuntime` behind `cfg(any(test, feature = "testing"))` as in "The fake": shared state,
  `exit_with`, `set_logs`, `fail_next`, `fail_always`, `clear_failures`, `calls()`. Its unit tests cover every
  scenario of the spec on the fake: repeated create, start after an exit or a failure, stop of a running container
  (`Exited` with 143) and of a created one, remove and stop of a missing one, `copy_in` outside `Created`, repeated
  ensure, an internal-flag conflict, a sweep by label and a sweep that skips a network in use, scripted exit and
  failure. Verify: `cargo test -p cicada-containers` and `cargo test -p cicada-containers --features testing`.
- [x] 3. **The gate.** `scripts/ci/feature-builds.sh`: `cargo check -p cicada-containers` with and without
  `--features testing`. `scripts/ci/dependency-trees.sh`: no library crate (`cicada`, `cicada-test`, `cicada-axum`,
  `cicada-client`, `cicada-otel`, `cicada-actors`, `cicada-app`) has `cicada-containers` in its tree (ADR 0056).
  Verify: `bash scripts/ci/feature-builds.sh`, `bash scripts/ci/dependency-trees.sh`, and the script tests
  `python3 -m unittest discover -s scripts -p 'test_*.py'`.
- [x] 4. **Version, changelog and docs handoff.** Set 0.20.7 in every version place, as a patch release does
  (`builder.md`), with a `## 0.20.7 — unreleased` changelog section naming the new crate and ADR 0056. Write
  `docs-handoff.md` from "Docs it breaks": nothing blocks; the AGENTS.md map row, `README.md#layers`,
  `docs/how-to/install.md#choose-the-crates` and the crate table of `docs/architecture.md`. Verify:
  `cargo test -p cicada-examples --test skill` and `--test docs`.
- [x] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-runtime --strict`, commit, then `python3 scripts/gate.py run`
  (no E2E: E2E runs at the release).
