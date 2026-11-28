# Tasks

Fast lane: tests only, in the unreleased batch (0.26.0; check the `release:` line of `python3 scripts/status.py`,
builder.md, Version). No design or spec deltas; follow `proposal.md`, What Changes. Touch only
`examples/tests/reply.rs` and `docs-handoff.md`; tests only, so no CHANGELOG line.

- [x] 1. **Fakes for the port.** In `examples/tests/reply.rs`, two fake actors with `type Ref = Pricing` (from
  `cicada_examples::reply`): one that answers every `QuoteRequest` with `Quote` at a fixed unit price, one that
  answers with `QuoteRejected`. Each replies to `message().from` with `send_to` and the request's correlation id, as
  the real pricing actor does, and imports nothing of the cart. Tests on virtual time host the real `CartActor` with
  one fake each (not `PricingActor`) and check the cart's priced lines and its rejected line. A short comment above
  the fakes says what they show: the cart depends only on the pricing port. Verify:
  `cargo test -p cicada-examples --test reply`.
- [x] 2. **The reply family match.** One test encodes each variant of `PricingReply` with `cicada::message::encode`
  and decodes it as `CartMsg` with `cicada::message::decode`, and checks the decoded fields. A comment says why: the
  match between a port's reply family and a caller's message family is checked only at runtime. Verify:
  `cargo test -p cicada-examples --test reply`.
- [x] 3. **Docs handoff.** `docs-handoff.md` asks for a section in `docs/guide/design.md` on ports and fakes, with the
  points of the proposal's What Changes, linking the new tests; and an actor-design skill route if the docs session's
  new heading needs one. Nothing blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --test docs`.
- [x] 4. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate actor-ports --strict`, commit, then `python3 scripts/gate.py run`.
