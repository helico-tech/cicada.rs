# Tasks

Normal lane, additive: in the unreleased 0.24.0 batch (check the `release:` line of `python3 scripts/status.py`,
builder.md, Version). Builds on `placement-asks` (holders and the line; lands first). Follow `design.md`; the spec is
`specs/placement-pool/spec.md`. The code is `crates/cicada-actors/src/placement.rs`.

- [x] 1. **Watches settled per turn.** At the end of each turn, the targets of holders and waiting asks before and
  after:
  `watch_existing` for new ones, `unwatch` for those that hold and wait for nothing (decisions 1 and 3); an ask whose
  `to` is the pool itself refused for good. Verify: tests that a queued ask's target is watched, that a release and a
  line grant to one target leave it watched, and that the last release unwatches.
- [x] 2. **Stopped and Failed.** A `Stopped` for another actor frees its places and removes its waiting asks once, then
  the line is scanned; for the pool itself or an actor that holds nothing, nothing; a `Failed` about another actor,
  nothing (decision 4). Verify: the deleted-before and deleted-after tests, two holders of one target, a holder's
  failed message.
- [x] 3. **Changelog and docs handoff.** The 0.24.0 section names freed places of deleted holders under Added; the
  handoff
  carries the design's Docs section and the turn a holder's `Failed` costs. No edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Verify: `cargo test -p cicada-actors`, `cargo test -p cicada-examples --test docs`.
- [ ] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-holders --strict`, commit, then `python3 scripts/gate.py run`
  (in the background with a 2-hour timeout).
