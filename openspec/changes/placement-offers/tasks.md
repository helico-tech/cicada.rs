# Tasks

Normal lane, additive: in the unreleased batch after 0.23.x; the placement line is released as one minor version
once `placement-drain-and-gone` has landed (decision 6), so do not mark anything released. Check the `release:` line of
`python3 scripts/status.py` for the version (builder.md, Version). Follow `design.md` and ADR 0067
(`docs/adr/0067-a-placement-pool.md`); the spec is `specs/placement-pool/spec.md` (a new capability).

- [ ] 1. **Types and state.** `crates/cicada-actors/src/placement.rs`: `Placement` (the ref, `cicada.placement/<pool>`),
  `PlacementMsg` (`Offer`, `Check`, `Rearm`, `Alert`, `Alerts`) with checked constructors (`offer(node, instance,
  labels, places)`), `PlacementState::new(silence, max_nodes)` with a checked decode (as `PermitsState`), the node
  entries (instance, labels sorted and deduplicated, places, heard at, live or silent), `PlacementNotice`. Export from
  `lib.rs`. Verify: unit tests of the bounds and the decode.
- [ ] 2. **Offers.** An offer replaces the node's entry and records the turn's store time; a new node past `max_nodes`
  and out-of-bounds messages are refused for good (decision 3). Verify: actor tests on virtual time with `TestActors`.
- [ ] 3. **Silence and alerts.** The self-only `Check` armed at the earliest `heardAt + silence` (kept in `armed`),
  `Rearm`, silent and live nodes, one `Alert` per changed node in its own turn sending `PlacementNotice` to the target
  set by `Alerts` (decision 4). Verify: the silence, return, lost check, foreign check and failing target tests.
- [ ] 4. **Contract, changelog and docs handoff.** Add the pool to `the_twelve_contracts()` (and its name test) in
  `crates/cicada-actors/tests/wire_formats.rs`, its reference JSON under `docs/reference/contracts`
  (`CICADA_UPDATE_CONTRACTS=true`) and message and state samples. The unreleased section of `CHANGELOG.md` names the
  pool under Added. `docs-handoff.md` carries the design's Docs section; nothing blocks. No edit to `README.md`,
  `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-actors`, `cargo test -p cicada-examples --test
  docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-offers --strict`, commit, then `python3 scripts/gate.py run`.
