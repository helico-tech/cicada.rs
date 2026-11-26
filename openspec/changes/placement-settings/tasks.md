# Tasks

Normal lane, in the 0.25.0 batch (check the `release:` line of `python3 scripts/status.py`, builder.md, Version).
Builds on `placement-resource-key` (landed). Follow `design.md`; the spec is `specs/placement-pool/spec.md`. No ADR.

- [x] 1. **The message.** `PlacementMsg::Configure { silence, max_nodes, max_waiting, max_holders }`, each optional
  (silence an ISO-8601 duration on the wire), with the create's bounds; an empty one or a value out of bounds refused
  for good; applied to a copy that must pass the full state check (decisions 1 and 3). Verify: unit and wire tests.
- [x] 2. **Limits that keep what is there.** The state check bounds the nodes, holders, waiting asks and contests by
  the hard cap of 1,000 instead of their limits; the rustdoc of `PlacementActor`, `PlacementState` and `RawState`
  updated (decision 2). Verify: the pool tests of the design for a lower and a higher limit, and the decode tests.
- [x] 3. **The silence.** A different silence with a check armed re-arms at the new earliest end; an equal one does
  nothing (decision 3). Verify: a shorter silence marks a quiet node silent with an alert in the next turn; a longer
  one moves the armed check later.
- [x] 4. **Contract, changelog and docs handoff.** `CICADA_UPDATE_CONTRACTS=true` regenerates the contract reference and
  samples; the 0.25.0 section names the message under Added; `docs-handoff.md` carries the design's Docs section. No
  edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test docs` and
  `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-settings --strict`, commit, then `python3 scripts/gate.py run`
  (wait for the lock with short tries; the run as its own background command with a 2-hour timeout).
