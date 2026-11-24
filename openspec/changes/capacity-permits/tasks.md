# Tasks

Normal lane, additive: the version of the unreleased batch (0.20.7), or the next patch if 0.20.7 is released before
the build starts. Follow `design.md` and ADR 0058; the spec is `specs/permit-pool/spec.md`. Runtime behaviour is
tested on virtual time (`#[tokio::test(start_paused = true)]` with `TestActors`).

- [ ] 1. **Types, state and decoder.** In `crates/cicada-actors/src/permits.rs`: `Permits` (`cicada.permits`,
  `Permits::named`), `PermitsMsg` (`Acquire`, `Release`, `Cancel`, `Resize`, `Audit`, the internal `Notify`, and
  `Failed`/`Stopped` for the watch on itself) with the `acquire` helper, `PermitsState` with `new(capacity,
  max_waiting)`, its holders (with `grant` and `notified`) and waiting requests, the decoder's checks (decision 2),
  `PermitNotice` with its four kinds, and the error type; exported from `lib.rs`, with its row in the rustdoc table.
  Verify: unit tests of the constructor and the decoder's bounds (capacity, request IDs, unique IDs, grants below
  `nextGrant`, `nextGrant >= 1`, `waiting.len() <= maxWaiting`); `cargo test -p cicada-actors --lib`.
- [ ] 2. **The handler.** `Acquire` answered in its commit (granted, queued, refused; repeated acquires), `Release`
  with fencing, `Cancel` of waiters only, `Resize`, `Audit`, the FIFO grant loop after every message, `Notify` sent
  and acted on only by the pool itself, the watch on itself and the freeing of an undelivered grant (decisions 4–11),
  and the refusals for good of decision 9 with the state unchanged. Verify: `cargo test -p cicada-actors`;
  `cargo clippy -p cicada-actors --all-targets`.
- [ ] 3. **Virtual-time tests.** `crates/cicada-actors/tests/permits.rs`, one test or more for every scenario of the
  spec, including the deleted waiter whose permit comes back, the failed `check` that keeps its permit, the foreign
  `Notify` and `Failed`, and each refusal. Verify: `cargo test -p cicada-actors --test permits`.
- [ ] 4. **Contract and wire format.** `cicada.permits` joins the contracts checked in
  `crates/cicada-actors/tests/wire_formats.rs` (ten become eleven), with samples of each message, a state with
  holders and waiting requests, and a notice of each kind taken by the schemas and the decoders;
  `CICADA_UPDATE_CONTRACTS=true cargo test -p cicada-actors --test wire_formats` writes
  `docs/reference/contracts/cicada.permits.json`, which is committed. Verify:
  `cargo test -p cicada-actors --test wire_formats`.
- [ ] 5. **Changelog and docs handoff.** The `0.20.7 — unreleased` changelog section names the permit pool and ADR
  0058. `docs-handoff.md` from "Docs it breaks" in the design; nothing blocks. No edit to `README.md`, `AGENTS.md` or
  the handwritten docs. Verify: `cargo test -p cicada-examples --test docs`, `cargo test -p cicada-examples --test skill`.
- [ ] 6. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate capacity-permits --strict`, commit, then `python3 scripts/gate.py run`
  (no E2E: no sample service, HTTP crate or app command changes; E2E runs at the release).
