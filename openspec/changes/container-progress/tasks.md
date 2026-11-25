# Tasks

Normal lane, **BREAKING** (the contract of every container job type changes): a new `## 0.23.0 — unreleased` section
in `CHANGELOG.md` above 0.22.1, and 0.23.0 wherever the version lives (builder.md, Version). Follow `design.md` and ADR
0065 (`docs/adr/0065-container-progress-through-a-file.md`, on main); the spec is `specs/container-actor/spec.md`. Run
the crate's tests with every feature: `cargo test -p cicada-containers --all-features`.

- [x] 1. **The protocol.** `Start` and `EndTarget` gain `progress_type: Option<String>` (serde `progressType`, checked
  like `messageType`); `ContainerMsg::start_with_progress`; `ContainerProgress { key, seq, text }` with its schema;
  `ContainerJob::PROGRESS_FILE` (default `/tmp/cicada-progress`); state fields `progress_seq` and `progress_hash`
  (SHA-256 hex; `sha2` no longer optional). Update the test job's contract snapshot
  (`CICADA_UPDATE_CONTRACTS=true`). Verify: wire and schema tests; `cargo test -p cicada-containers --all-features`.
- [x] 2. **The create.** With `progressType`: the path rule of `copy_in` on `PROGRESS_FILE` (a refused path ends the
  container `failed`/`notStarted`, nothing made) and `CICADA_PROGRESS_FILE` set after the job's `spec` (decision 2).
  Verify: actor tests for the env, a refused path and no `progressType`.
- [x] 3. **Reading and sending.** At each running poll and in the turn that records an end the container ran for
  (decision 3): `copy_out` with 64 KiB and one file; lossy UTF-8 with NUL as U+FFFD; an empty text only after a
  non-empty one; on a new hash, `seq` + 1 and a `ContainerProgress` send with the end's retry policy; a refused send
  moves `seq` and is not retried; failures send nothing and do not count. The rustdoc names atomic writes, the tmpfs
  warning and dropping late progress by `seq` (decisions 4 and 5). Verify: the tests of the design's Tests section.
- [x] 4. **Changelog and docs handoff.** The 0.23.0 section names the progress protocol under Added and the contract
  change under a **BREAKING** line with "Changing a contract" for container job types and the Rust fields. The
  handoff carries what the design's Docs section lists; nothing blocks. No edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc` (the container guide's blocks
  compile: if a block builds `Start` or `EndTarget` and stops compiling, report `Docs: blocked`).
- [x] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate container-progress --strict`, commit, then
  `python3 scripts/gate.py run`.
