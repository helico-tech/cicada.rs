Verdict: pass

## Blocking findings

None.

## Notes

1. `examples/tests/reply.rs:106` and `:127`: the two fake tests do not assert `test.failures().await.is_empty()`, as the real-pricing test at `:103` does. Their state checks already cover the outcome, so this is optional. It would also catch a fake whose reply failed to send.
2. `openspec/changes/actor-ports/proposal.md` (What Changes, last bullet) says "a fake's contract hash differs". The handoff's caveat is right and the proposal is not: `Contract::new` (`crates/cicada/src/contract.rs:34`) hashes the type name, the state schema and the message schemas. Both fakes use `NoState` and `Ref = Pricing`, so they get the real pricing actor's hash. The docs session should follow the handoff, not the proposal.

## Checked

- Tasks 1–4 are ticked, and the work is in the diff. The fakes are `impl Actor` with `Ref = Pricing`, inside a module that imports nothing of the cart. They reply with `send_to` to `message().from` and pass on the request's correlation id. The tests host the real `CartActor` on virtual time. The decode test covers both `PricingReply` variants.
- Scope: only `examples/tests/reply.rs` and the change's files changed. There is no public API change, and no protected doc or CHANGELOG edit (tests only, as the tasks say).
- Versions are 0.26.0 in `Cargo.toml`, both skills and `.claude-plugin/plugin.json`.
- The handoff follows the template. Its links resolve, and the "Design protocols" heading exists in `docs/guide/design.md`. `App::host`, `TestActors::builder().with` and `ContainerRuntime`/`FakeRuntime` exist. Its status claims no prose that is not written.
- The builder's example command matches the test file (6 tests).

## Not checked

I ran no tests (as the brief says) and did not verify the gate evidence for 7047227a. There are no spec scenarios, because this is a fast-lane change.
