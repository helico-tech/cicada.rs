# Tasks

Normal lane: opens the 0.25.0 batch (a new `## 0.25.0 — unreleased` section; a contract change of the pool and a new
field on `NodeConfig`; builder.md, Version). Follow `design.md` and ADR 0071
(`docs/adr/0071-a-duplicate-node-reported-by-its-member.md`); the spec is `specs/placement-pool/spec.md`.

- [x] 1. **The node's host name.** `NodeConfig::hostname: Option<String>` (`None`: the machine's own, as today, through
  the private `usable_hostname`), written to the node row; `ActorSystem::hostname()` returns the value written
  (decision 5). Verify: a test that the row and the getter hold an override; `cargo test -p cicada`.
- [ ] 2. **The identity lock.** `load_or_create` locks `dir/placement-identity.lock` (`File::try_lock`) before it reads
  or creates the file; the lock lives in an `Arc` inside `NodeIdentity` (still `Clone`; equality ignores it);
  `PlacementMember` keeps the whole identity; `InvalidIdentity::kind()` with `IdentityErrorKind::{Invalid, Locked}`; a
  file system without locks warns and goes on (decisions 3 and 4). Existing tests that load twice drop the first
  identity first. Verify: the lock tests of the design.
- [ ] 3. **The duplicate check and the pool.** Each round, first, the member reads `ActorSystem::node(id)` and compares
  its host name with `ActorSystem::hostname()`; on a difference: no offer, one error log per duplicate, `Duplicate {
  node, host }` to the pool. The pool: the mark's optional `host`, one `conflict` alert per mark, no new places while
  marked, the end rules of ADR 0071; unknown nodes refused for good; contract reference and samples regenerated
  (decisions 1 and 2). Verify: the duplicate tests of the design (two systems under one id with different hostnames).
- [ ] 4. **Changelog and docs handoff.** A new `## 0.25.0 — unreleased` section with Added (duplicate check, lock,
  `NodeConfig::hostname`) and the upgrade note for `NodeConfig` struct literals; the handoff carries the design's Docs
  section and the limits. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --test docs` and `--doc` (report `Docs: blocked` if a guide block builds a `NodeConfig`
  literal or uses `NodeIdentity` in a way that stops compiling).
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace` with PostgreSQL, `openspec validate placement-identity-lock --strict`, commit, then
  `python3 scripts/gate.py run` (wait for the lock with short tries; the run as its own background command with a
  2-hour timeout).
