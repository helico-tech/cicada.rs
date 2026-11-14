## Implementation

- [x] 1. Add the design's marked correction notes to the archived `2026-11-14-deep-failure-notices/design.md`
  export-conversion paragraph and `tasks.md` task 3. Preserve every existing line and checkbox; the design note also
  qualifies the Proofs section's broad schema/serde wording. Do not edit the archived spec delta or other archives.
  Verify that the two file diffs contain added lines only, the original claims remain verbatim, and no added note
  changes checkbox/task counts. Keep the task note indented inside task 3 with no new checkbox.
- [x] 2. Narrow only the schema-agreement sentence in the rustdoc of `check` in
  `examples/src/dead_letters_upgrade.rs`: neither it nor `create_actor` evaluates JSON Schema, and the cited test
  proves agreement only for the tested payload fields, not the whole state. Keep the four existing checks described;
  change no function body, signature, test logic or fixture. Run `cargo test -p cicada-examples --test dead_letters_upgrade`,
  `cargo test -p cicada-actors --test dead_letters the_schema_and_the_decoder_agree_on_the_payload_fields_of_a_letter`
  and `cargo doc -p cicada-examples --no-deps`; inspect rustdoc output for new warnings.
- [ ] 3. Write `docs-handoff.md` under the predecessor's rules: report the corrected archive/rustdoc claims, the
  evidence and checks; no new guide work and Blocking: none because `docs/how-to/upgrading.md` already describes the
  pre-deletion checks accurately. Supply the release version for docs-owned installation pins. Add a correction-only
  changelog entry and the normal next patch version chosen at build time, including skill metadata and required
  generated version updates. Do not claim a docs-only release exemption or edit protected prose. Verify with
  `cargo test -p cicada-examples --test docs`, `cargo test -p cicada-examples --test skill` and
  `python3 scripts/status.py`; confirm unchanged queue order and all six scenarios in the spec delta.

## Finishing checks

- [ ] 4. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate dead-letters-upgrade-wording --strict`, with PostgreSQL where configured and skipped checks
  recorded. Inspect the final diff for comment/artifact/release-only scope: no behavior, API, contract hash, schema
  or guarantee change. Follow normal build-loop review and owner-approved landing/release gates.
