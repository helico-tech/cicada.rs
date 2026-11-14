# Docs handoff for dead-letters-upgrade-wording

Version: 0.18.7. Refresh the dependency pins of `docs/how-to/install.md` once it is published. No API, wire format,
contract hash, store schema, guarantee, CLI or HTTP route changes.

## What changed

No new guide work. The release corrects claims, not behaviour:

- The doc comment of `check` in `examples/src/dead_letters_upgrade.rs` (the worked example of the 0.17.0 dead-letter
  upgrade) said the decoder agrees with the JSON Schema "on the fields of a letter". It now says that neither `check`
  nor `ActorSystem::create_actor` evaluates the JSON Schema, and that the test
  `the_schema_and_the_decoder_agree_on_the_payload_fields_of_a_letter` (`crates/cicada-actors/tests/dead_letters.rs`)
  covers the payload fields of a letter only, not the rest of the state. The four checks it lists are unchanged.
- The archived `openspec/changes/archive/2026-11-14-deep-failure-notices/design.md` and task 3 of its `tasks.md`
  promised validation "against the new schema/limits". Each now carries a marked correction note; no original line
  changed.
- The spec delta clarifies the requirement `A breaking dead-letter upgrade preserves exported letters explicitly`: the
  checks before deletion are named, and they are not JSON Schema validation.

Why no prose: `docs/how-to/upgrading.md`, `Version 0.17.0`, steps 5 and 6, already describes `check` as the checks
`restore` needs (no NUL, at most 64 levels, the state type of 0.17.0) and nowhere claims JSON Schema validation.

## Blocking

None. Every doctest, link, guarantee row and skill route passes without prose edits.

## Status

Checks run on the code branch: `cargo test -p cicada-examples --test dead_letters_upgrade` (5 passed), the
payload-fields test (1 passed), `cargo doc -p cicada-examples --no-deps` (no warnings), `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings` (clean), `openspec validate dead-letters-upgrade-wording
--strict` (valid) and `cargo test --workspace` with PostgreSQL (2905 passed, 0 failed). After the rebase onto the docs
commit `a5980e7`: `cargo test -p cicada-examples --doc` (28 passed), `--test docs` (11 passed) and `--test skill`
(8 passed). Optional, not required: step 5 of the 0.17.0 upgrade lists three of the four checks; the docs session may
add "and no address is in the export twice". Nothing above is written in docs-owned files, and no owner fix is
integrated.
