# Tasks

Fast lane (rustdoc only), version 0.26.0 (unreleased). Its crate is `cicada` only, so it can build next to an inspector
change. Follow `proposal.md`; ADR 0015 and the store's code are the source for what `caused_by` holds.

- [ ] 1. **The rustdoc.** `MessageView::caused_by` in `crates/cicada/src/api/mod.rs`, and any store-side field or
  method doc that says the same (search `caused_by` and "whose turn sent"), name what the store records: the message
  whose turn sent it; the original of an operator retry; for a `Stopped` notice, the message of the turn that stopped
  or deleted the actor, or of the turn whose `watch_existing` found it absent; none for a send from outside an actor,
  a `Failed` notice or a stop from outside a turn. Check each case against the store's code before writing it. Verify:
  `cargo doc -p cicada --no-deps` without warnings.
- [ ] 2. **The generated reference.** `CICADA_UPDATE_OPENAPI=true cargo test --workspace` regenerates
  `docs/openapi.json` where the description comes from the rustdoc; commit what it writes. Verify: `cargo test -p
  cicada-axum` (the OpenAPI drift test).
- [ ] 3. **Finishing checks.** `cargo fmt --all --check` (lines ≤120, comments included), `cargo clippy --workspace
  --all-targets`, `cargo test --workspace`, `openspec validate caused-by-doc`, commit. No changelog entry (rustdoc
  only) and no docs handoff unless a guide quotes the old wording (then name it in `docs-handoff.md`). The change loop
  runs the gate at landing.
