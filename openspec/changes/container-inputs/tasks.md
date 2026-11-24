# Tasks

Normal lane, additive: 0.20.9, with the rest of the container line, released when the line, through
`container-limits-and-kinds`, has landed (check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Builds on `container-make-dirs` (`CopyDir`, `make_dirs`) and `container-consistency` (the shared path
rule), which must have landed first. Follow `design.md`; the spec is `specs/container-actor/spec.md`. Runtime
behaviour is tested on virtual time (`#[tokio::test(start_paused = true)]` with `TestActors`) and the fake.

- [x] 1. **The job method and the check.** In `crates/cicada-containers/src/actor/mod.rs`: `Inputs { dirs, files }`
  (`non_exhaustive`, `Default`), exported from `lib.rs`; `ContainerJob::inputs` with a default of no inputs
  (decision 1). Its rustdoc names when it is called (at each create, never stored, sync), the order (folders, then
  files, then start), the refusals of decisions 3 and 4, that an `Err` on a replayed step leaves a running container
  running until `Remove`, as `spec` does, and the secret-file advice of decision 5. A private check of decision 3:
  every path parsed by the shared path rule, a file at or above a folder refused. Verify: unit tests of the check
  (a file at a folder, at a parent, `/out/.` with `/out`, a path parsing refuses, folders and files side by side);
  `cargo test -p cicada-containers --all-features`.
- [x] 2. **The create step.** In `ContainerActor::create`: build the spec and the inputs, check the inputs, `create`,
  then with inputs `inspect` and, only when `Created`, `make_dirs` (with folders) and `copy_in` (with files), then
  `start` (decision 2); `Missing` after `create` counts an error. Failures by decision 4: `InvalidSpec` ends
  `failed`; `Conflict` inspects again (`Created` ends `failed`, anything else counts an error); `Other` from
  `make_dirs` ends `failed` at once; other errors count. Without inputs the calls stay `create` and `start`. Verify:
  `cargo test -p cicada-containers --all-features`; `cargo clippy -p cicada-containers --all-targets --all-features`.
- [x] 3. **Virtual-time tests.** In the actor's tests (`crates/cicada-containers/tests/actor.rs`), one test or more
  for every scenario of the spec delta: inputs before start (calls in order, nothing of the inputs in the stored
  state), no inputs, a replayed step after start, inputs that cannot be built (all four cases), a refused input, a
  conflict while copying (still `Created`, and `Missing`), a runtime without `make_dirs` (a fake wrapper that keeps
  the trait's default), a container gone after create, an unavailable daemon while copying. Verify:
  `cargo test -p cicada-containers --test actor`.
- [x] 4. **Changelog and docs handoff.** The 0.20.9 section of `CHANGELOG.md` names `ContainerJob::inputs` and
  `Inputs` under Added, and that the actor now puts inputs in between create and start. `docs-handoff.md` carries
  `inputs`, `Inputs`, the order, the refusals and the secret-file advice for the container guide; nothing blocks. No
  edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-inputs --strict`, commit, then
  `python3 scripts/gate.py run` (no E2E: the actor runs on the fake and no task touches `e2e/`; E2E runs at the
  release).
