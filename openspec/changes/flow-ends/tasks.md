# Tasks

Normal lane, **BREAKING**: in the version the change loop names (the one after 0.26.0, shared with
`flow-request-target`; design decision 7). Follow `design.md`; the specs are `specs/flows/spec.md` and
`specs/flow-finalizer/spec.md`; ADR 0080 is committed with the change. Run the crate's tests: `cargo test -p
cicada-actors`.

- [ ] 1. **The output.** `type Output` on `Flow`, `run` returning `Result<Self::Output, HandlerError>`;
  `FlowState<I, O = ()>` with `output`, saved when the run is done, checked like a step's result (a failed check is a
  refusal), decoded from `null` for a done run (decisions 1 and 2). Fix every `Flow` impl and `FlowState::new` in the
  workspace (`event.rs`, the examples, the sample service, `cicada-otel`'s test); the examples get `type Output = ();`
  or a real output where it shows something. Verify: tests for a saved output and a `()` output read back; `cargo test
  --workspace`.
- [ ] 2. **The failed end.** `FlowStatus::Failed` and `failure`; the flow code's errors end the main code when permanent,
  without a retry policy, or at `failures + 1` reaching the policy's attempts; that turn keeps the reason, marks the run
  cancelling and arms the cleanup as a `Cancel` turn does; a successful cleanup ends `failed`; cleanup code that fails
  for good ends `failed` at once with `cleanup: `; the reason cleaned and cut; refusals, apply failures, killed turns
  and panics as today; a failed run ignores `Continue`, `Reply` and `Cancel` (decisions 3 to 5). Verify: the tests of
  decision 8 on virtual time with `TestActors`; `cargo test -p cicada-actors`.
- [ ] 3. **The contract.** Decode checks the new invariants (decision 6); regenerate the test flow's contract, the sample
  service's `flows.onboarding` snapshot and any contract reference with `CICADA_UPDATE_CONTRACTS=true cargo test
  --workspace`, and commit what it writes. Verify: decode tests for each broken invariant; `cargo test -p cicada-actors
  --test wire_formats`.
- [ ] 4. **Changelog and docs handoff.** The unreleased section of `CHANGELOG.md` (the version the change loop named)
  gets the change under Changed with **BREAKING**: `type Output`, `FlowState`'s parameter, `FlowStatus::Failed`, no
  `Failed` notice for a flow-code failure, the contract change and ADR 0080. `docs-handoff.md` lists every doc block
  that stops compiling as blocking (design, Docs this breaks), plus the statuses text, the watch advice, the
  architecture paragraph and the upgrade note. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify:
  `cargo test -p cicada-examples --doc` and `--test docs`; report `Docs: blocked` with the failing blocks.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate flow-ends --strict`, commit, then `python3 scripts/gate.py run --e2e`
  (the sample service's contract changes).
