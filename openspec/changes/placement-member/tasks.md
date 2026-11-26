# Tasks

Normal lane, additive inside the unreleased 0.24.0 placement batch (check the `release:` line of
`python3 scripts/status.py`, builder.md, Version). Builds on `placement-conflicts` (lands first). Follow `design.md`;
the spec is `specs/placement-pool/spec.md`. Code in `crates/cicada-actors/src/placement/` (or next to `placement.rs`).

- [ ] 1. **The identity.** `NodeIdentity::load_or_create(dir)` (creates the folder and `placement-identity.json` with
  `node-<16 hex>` and a random instance, written atomically; reads it after; `InvalidIdentity` for an unreadable or
  invalid file, leaving it as it is), `node_id()` and `instance()` (decision 3); the rustdoc of `Offer::instance` says
  it is kept across restarts. Verify: tests in a temp folder.
- [ ] 2. **The member.** `MemberSettings` (interval 15 s, zero refused), `Capacity { labels, places }`,
  `PlacementMember::new(pool, identity, settings, probe)` and `run(system, stop)`: each interval the probe raced
  against `stop`; `Some` sends `Offer { node: system.node_id(), instance, labels, places }`, `None` nothing; a failed
  send logged and retried next round (decisions 1 and 2). Verify: actor tests on virtual time with `TestActors`.
- [ ] 3. **Changelog and docs handoff.** The 0.24.0 section names the member and the identity under Added; the handoff
  carries the design's Docs section (with Docker's `info()` as the probe example). No edit to `README.md`,
  `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-actors`, `cargo test -p cicada-examples --test
  docs`
  and `--doc`.
- [ ] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-member --strict`, commit, then `python3 scripts/gate.py run`
  (in the background with a 2-hour timeout).
