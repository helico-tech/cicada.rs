# Tasks

Normal lane, in the 0.25.0 batch (check the `release:` line of `python3 scripts/status.py`, builder.md, Version).
Builds on `placement-identity-lock` (lands first). Follow `design.md` and ADR 0072
(`docs/adr/0072-one-live-holder-per-resource.md`); the specs are `specs/placement-pool/spec.md` and
`specs/container-runtime/spec.md`.

- [x] 1. **The daemon's id.** `RuntimeInfo::id: Option<String>`, `RuntimeInfo::with_id`; `DockerRuntime::info` reads
  `/info`'s `ID` (empty: `None`) next to `/version`; the fake reports `None` until `set_info` sets one; the rustdoc
  names the cost of `/info` (decision 1). Verify: map and fake tests; `cargo test -p cicada-containers --all-features`.
- [x] 2. **The key on offers.** `Capacity::resource(key)`, `Offer::resource` (1 to 200 characters, no NUL), the node's
  key in the pool state (decision 2). Verify: wire and bounds tests.
- [x] 3. **One live holder.** The `contested` list, one alert per added entry, removal by the check timer one silence
  time after `at`, the key taken from a silent or gone holder, the decode refusing two live holders; contract reference
  and samples regenerated (decision 3). Verify: the pool tests of the design.
- [x] 4. **E2E, changelog and docs handoff.** The container E2E class checks `info()`'s `id` is set. The 0.25.0 section
  names the id, the key and the rule under Added; the handoff carries the design's Docs section. No edit to
  `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-resource-key --strict`, commit, then
  `python3 scripts/gate.py run --e2e` (the change touches `e2e/`; wait for the lock with short tries, the run as its own
  background command with a 2-hour timeout).
