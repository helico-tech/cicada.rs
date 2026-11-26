# Tasks

Normal lane, in the 0.25.0 batch (check the `release:` line of `python3 scripts/status.py`, builder.md, Version).
Builds on `placement-settings` (lands first; same file). Follow `design.md`; the spec is `specs/placement-pool/spec.md`.

- [ ] 1. **ADR 0075.** `docs/adr/0075-a-full-pool-says-so-once.md` (extends ADR 0067, decision 5) and its row in
  `docs/adr/README.md`. Verify: `cargo test -p cicada-examples --test docs`.
- [ ] 2. **The mark.** `PlacementState::full: Option<PlacementFull { node, at }>` (`#[serde(default)]`), checked on
  decode; `next_check`, the armed invariant, `Check` and placement-settings' re-arm include it; the rustdoc of `Check`,
  `Rearm`, `armed` and `PlacementActor` updated (decisions 1 and 3). Verify: decode tests; the check ends the mark and
  clears `armed` when nothing else is left.
- [ ] 3. **The offer and the notice.** After the resource check, a new node's offer to a full pool commits, adds
  nothing, sets or moves the mark, arms the check when none is armed, and queues one `full` alert for a new mark; an
  offer that adds a node clears the mark. `PlacementNoticeKind::Full`, and the enum becomes `#[non_exhaustive]`
  (decisions 1, 2 and 4). Verify: the pool tests of the design on virtual time.
- [ ] 4. **Contract, changelog and docs handoff.** `CICADA_UPDATE_CONTRACTS=true` regenerates the contract reference and
  samples; the 0.25.0 section names the notice under Changed with the `#[non_exhaustive]` kind and the rolling-upgrade
  note; `docs-handoff.md` carries the design's Docs section. No edit to `README.md`, `AGENTS.md` or the handwritten
  docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-full-notice --strict`, commit, then
  `python3 scripts/gate.py run` (wait for the lock with short tries; the run as its own background command with a
  2-hour timeout).
