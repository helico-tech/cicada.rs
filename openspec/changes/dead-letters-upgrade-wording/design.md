# Design

## Correct the claims, preserve the history

The built upgrade example already checks the intended restore data. Its `check` refuses NUL in strings/keys, state
depth above 64, failure to decode as `DeadLettersState` (including null), and repeated export addresses. `convert`
runs it and `reset` repeats it before deletion. Neither it nor the core `create_actor` evaluates JSON Schema.
HTTP validation remains separate, as architecture and ADR 0026 describe. No runtime, API, contract or store change.

Add a marked correction after the archived design's export-conversion paragraph, changing no existing line. It says:

> **Correction (added by `dead-letters-upgrade-wording`).** The built example does not validate against the JSON
> Schema, and neither does `create_actor`, which `restore` calls. Before anything is deleted, `check` (run by
> `convert` and again by `reset`) refuses NUL in strings or keys, a state deeper than 64 levels, a state that does not
> decode as the 0.17.0 state, and an address repeated in the export. The test
> `the_schema_and_the_decoder_agree_on_the_payload_fields_of_a_letter` covers the payload fields of one letter only.
> It does not prove schema/decoder agreement on the whole state. This also qualifies "schema/serde agreement" under
> Proofs and scope. The original wording is kept as written.

Under archived task 3 add an indented blockquote, not a checkbox:

> **Correction (added by `dead-letters-upgrade-wording`).** "validate converted states against the new
> schema/limits" was built as `check`: typed decode of the 0.17.0 state, the depth limit, NUL characters and repeated
> addresses. It is not JSON Schema validation. See the correction in `design.md`.

This preserves the record and completed task count. Reject replacing/striking out old sentences or editing the
archived spec delta. The extra Proofs qualification is the same overclaim in the same file, now explicit in scope;
leave task 2's parity statement in its already-specific payload-conversion context and other archives untouched.

## Narrow the rustdoc to its proof

Replace only `check`'s broad schema-agreement sentence. Keep the four listed checks and explain:

> `create_actor` does not check the JSON Schema of the contract, and neither does this. A test shows that the schema
> and the decoder agree on the payload fields of a letter (`crates/cicada-actors/tests/dead_letters.rs`); it does not
> show that they agree on the rest of the state.

The existing test covers both/neither fields, null text, raw null, text, marker-shaped raw values and an unknown field.
It is not a proof for every field. Leaving the current comment would keep the overclaim; adding schema validation or
new runtime tests would broaden a text correction. No function body, test logic, public signature or fixture changes.

## Spec and documentation ownership

Use a MODIFIED delta for `A breaking dead-letter upgrade preserves exported letters explicitly` in `dead-letters`.
Copy its complete current requirement and all six scenarios. Name the helper's four checks and explicitly exclude
JSON Schema validation. Clarify the legacy-deep scenario as the pre-deletion check accepting the converted state.
Do not claim every core create always performs typed decoding: the management node may host nothing, and the example
performs its own typed check. Preserve topology, quiescence, exports, identity, restore and redrive requirements.

No new ADR: this aligns text with existing architecture/ADR 0043 and changes no behavior or guarantee. The builder
owns the archived change artifacts and example rustdoc. `docs/how-to/upgrading.md` already lists the correct checks;
no protected prose needs an edit. The required handoff says no new guide work, with that evidence and Blocking: none,
while still giving the release version for docs-owned installation pins. Do not label the entire release no-impact:
its archive/rustdoc claims are corrected. Follow the predecessor's handoff rules and preserve queue order.

## Verification and release

Inspect the archive diff for added lines only, exact preservation of the old claims, and unchanged checkbox counts.
Run the existing upgrade integration test and the payload-fields schema/decoder test. Build example rustdoc with
`cargo doc -p cicada-examples --no-deps`; run docs/skill checks for release metadata. No new mirrored-prose test.
The existing upgrade tests cover stop-before-delete and restored legacy data; they support clarified existing behavior.

Three implementation tasks plus finishing suffice. Use the normal build-loop patch release, with a correction-only
changelog entry and version chosen at build time. This is not docs-session maintenance exempted by the owner. Keep
ordinary fmt/clippy/workspace/strict checks and review/release gates. The design stage runs strict validation only;
it does not claim the builder's tests have already run.
