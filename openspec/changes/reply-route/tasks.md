# Tasks

Normal lane: in the unreleased batch (0.26.0; check the `release:` line of `python3 scripts/status.py`, builder.md,
Version). Follow `design.md`; the spec is `specs/flow-replies/spec.md`; ADR 0079 is committed with the change. Run the
crate's tests: `cargo test -p cicada-actors`.

- [x] 1. **The type and its wire.** `crates/cicada-actors/src/reply.rs` with `ReplyRoute<T>` (`Flow(ReplyTo<T>)`,
  `Actor { to, correlation }`), exported from the crate root with a row in its table of items; hand-written `Clone`,
  `Debug`, `PartialEq`, `Eq` without bounds on `T`; `From<ReplyTo<T>>`, `ReplyRoute::actor(to)` and
  `.correlation(id)` (decision 1). Serde decided by the key and a schema named `ReplyRoute`, the same for every `T`
  (decision 3). The private schema struct `ReplyRoute` in `flow.rs` becomes `ReplyToSchema`. Verify: unit tests for
  both round trips, a saved `ReplyTo` decoding as `Flow`, `actor` with `run` or `request` failing, missing fields
  failing, unknown fields ignored, and equal schemas for two `T`s; `cargo test -p cicada-actors`.
- [x] 2. **`send`.** One `send(turn, &value)` with `ReplyTo::send`'s shape: the flow form is `ReplyTo::send`; the actor
  form encodes the value as a message synchronously (`Invalid`, nothing buffered, when it does not), then sends it to
  `to` with `send_to`'s checks, the route's correlation id or else the turn's, no retry policy, ordinary headers and
  retention (decisions 2 and 4). Rustdoc with a doctest that shows one port field serving both callers, and that the
  actor form needs a tagged message family. Verify: tests on virtual time with `TestActors` for each scenario of the
  spec (actor caller with a later reply and `c1`, flow caller with `r.into()`, `5` gives `Invalid`, a missing type
  name gives `SendRejected`); `cargo test -p cicada-actors`.
- [ ] 3. **Changelog and docs handoff.** The unreleased section of `CHANGELOG.md` gets an Added line for `ReplyRoute`
  and ADR 0079. `docs-handoff.md` asks for the actors guide's text on requests near `step.request`, the design guide's
  ports section (from `actor-ports`), and the upgrade note that moving a field from `ReplyTo<T>` to `ReplyRoute<T>`
  keeps saved messages but changes the contract hash. Nothing blocks. No edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Verify: `cargo test -p cicada-examples --test docs`.
- [ ] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate reply-route --strict`, commit, then `python3 scripts/gate.py run`.
