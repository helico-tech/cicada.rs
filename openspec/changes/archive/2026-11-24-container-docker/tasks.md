# Tasks

Normal lane, additive: the version of the unreleased batch (0.20.7, after `container-runtime`). Follow `design.md`;
the spec is `specs/container-docker/spec.md`. Pure parts (the status, error and hardening mappings, the secret scrub,
the tar and the mount check of `copy_in`) are unit tests that run without Docker; the rest is the E2E class.

- [x] 1. **Connect and create.** The `docker` feature and its dependencies (decision 3; `futures` from the workspace),
  `DockerRuntime::connect()` with the negotiated API version (decisions 1, 2), the exact-name lookup (decision 4),
  `create` as decision 5 (container first, then env checks, image inspect, pull, create), the hardening and limits
  mapping, the error mapping and the secret scrub (decision 10). Verify: unit tests of the mappings and the scrub;
  `cargo test -p cicada-containers --features docker`; `cargo clippy -p cicada-containers --features docker
  --all-targets`.
- [x] 2. **The other methods.** `start`, `stop`, `remove` (decision 6), `copy_in` (decision 7, the read-only mount
  rule), `inspect` with the status mapping, `logs` and `list` (decision 8), `ensure_network`, `ensure_volume`
  (decision 9) and `sweep`. Bring the fake in line where `container-runtime`'s review found it looser than Docker:
  its `copy_in` also refuses a path with `..` and a path on a read-only mount, and `CopyFile`'s `Debug` shows the path,
  mode and length, not the contents. Verify: unit tests of the status mapping, the tar and the mount check;
  `cargo test -p cicada-containers --features docker`.
- [x] 3. **The Docker test.** The new class in `cicada-e2e` as in the design's test section: every scenario of the spec
  against a real daemon, its label, its sweep before and after, and the cleanup after a panic. Verify:
  `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture` (the new class, then the whole suite).
- [x] 4. **Gate, version and docs handoff.** `scripts/ci/feature-builds.sh`: the crate with `--features docker`.
  `scripts/ci/dependency-trees.sh`: no `bollard` in the crate's default tree, no rustls or native-tls with `docker`.
  The `0.20.7 — unreleased` changelog section names the backend. `docs-handoff.md` from the design's handoff list;
  nothing blocks. Verify: `bash scripts/ci/feature-builds.sh`, `bash scripts/ci/dependency-trees.sh`,
  `python3 -m unittest discover -s scripts -p 'test_*.py'`, `cargo test -p cicada-examples --test docs` and
  `--test skill`.
- [x] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `cargo deny check advisories`, `openspec validate container-docker --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`).
