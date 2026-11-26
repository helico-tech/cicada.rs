# Tasks

Normal lane, additive: in the unreleased 0.24.0 batch with `placement-offers` (the placement line ships together;
check the `release:` line of `python3 scripts/status.py`, builder.md, Version). Follow `design.md` and ADR 0068
(`docs/adr/0068-asks-in-the-placement-pool.md`); the spec is `specs/placement-pool/spec.md`. The code is
`crates/cicada-actors/src/placement.rs`; follow `permits.rs` for the protocol.

- [ ] 1. **Types and state.** `Ask`, `Release`, `Cancel`, `Notify`, `Failed`, `Stopped` with checked constructors
  (`ask(holder, needs, to, message_type)`, `release(holder, grant)`, `cancel(holder)`); `PlacementAnswer` and its
  reasons; state fields `maxWaiting`, `maxHolders`, `nextGrant`, the line and holder records (holder, needs, to,
  messageType, grant, node, notified) with a checked decode (unique holder ids across nodes and the line, grants
  between 1 and `nextGrant`); `PlacementState::new(silence, max_nodes, max_waiting, max_holders)`. Verify: unit tests
  of the bounds and the decode.
- [ ] 2. **Asks and placement.** Grant, queue or refuse (decisions 2 and 3); the answer in the ask's commit; a repeated
  ask; `Release` matching holder and grant; `Cancel` of a waiting ask; the line scanned after every message, skipping
  asks that cannot be placed. Verify: actor tests on virtual time for each.
- [ ] 3. **Grants from the line.** A self `Notify` in its own turn; an undelivered grant freed through the self-watch
  when `notified` is false; other `Failed` notifications change nothing (decision 4). Verify: the gone-target and
  undelivered-grant tests.
- [ ] 4. **Contract, changelog and docs handoff.** Contract reference and samples updated
  (`CICADA_UPDATE_CONTRACTS=true`); the 0.24.0 section names asks under Added; the handoff carries the design's Docs
  section. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-actors`,
  `cargo test -p cicada-examples --test docs`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate placement-asks --strict`, commit, then `python3 scripts/gate.py run`
  (in the background with a 2-hour timeout).
