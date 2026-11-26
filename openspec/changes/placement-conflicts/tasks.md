# Tasks

Normal lane, additive inside the unreleased 0.24.0 placement batch (check the `release:` line of
`python3 scripts/status.py`, builder.md, Version). Builds on `placement-drain-and-gone` (lands first). Follow
`design.md` and ADR 0069 (`docs/adr/0069-a-second-machine-under-one-node-id.md`); the spec is
`specs/placement-pool/spec.md` (a modified requirement). The code is `crates/cicada-actors/src/placement.rs`.

- [ ] 1. **The mark.** The node field `conflict: {instance, at}`, allowed only on a live node by the decode; an offer
  with another instance under a live node changes no entry field and sets the mark; a new mark queues one self
  `Alert` of kind `conflict`; later conflicting offers update the mark only (decision 1). Verify: actor tests on
  virtual time.
- [ ] 2. **The end of the mark.** Cleared when the node goes silent, or when the recorded instance offers after no
  conflicting offer for one silence time; a silent node's entry taken over by any instance (decision 2). Verify: the
  replaced-machine and second-machine-stops tests, and a later conflict alerting again.
- [ ] 3. **Contract, changelog and docs handoff.** The contract reference and samples regenerated
  (`CICADA_UPDATE_CONTRACTS=true`); the 0.24.0 section names the conflict alert under Added; the handoff carries the
  design's Docs section and the limits. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-actors`, `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-conflicts --strict`, commit, then
  `python3 scripts/gate.py run` (in the background with a 2-hour timeout).
