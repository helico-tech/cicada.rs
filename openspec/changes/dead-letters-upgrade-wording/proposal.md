# Proposal

## Why

`deep-failure-notices` (0.17.0) promises that the upgrade of dead letters validates converted states "against the new
schema/limits" before anything is deleted: its design (`openspec/changes/archive/2026-11-14-deep-failure-notices/design.md`,
about line 73) and its task 3 (`tasks.md`, about line 23). The built `check` in `examples/src/dead_letters_upgrade.rs`
does not check the JSON Schema. It checks what `restore` refuses: NUL characters, the depth limit, the typed decode of
the 0.17.0 state and a repeated address. `create_actor`, which `restore` calls, does not check the schema either. The
review of round 2 found this sound but asked for the wording to match. The test
`the_schema_and_the_decoder_agree_on_the_payload_fields_of_a_letter` covers only the payload presence rule. It should
not be read as proving that the schema and the decoder agree everywhere.

## What Changes

- The archived design and task 3 of `deep-failure-notices` say the upgrade checks the decoder and the store's limits
  before deletion, not the schema. The original wording stays readable as a marked correction.
- The doc comment of `check` keeps naming the schema/decoder test as covering the payload fields of a letter only.

## Out of scope

Checking the JSON Schema in the upgrade; any change to `check`, `convert`, `reset` or `restore`; other archived
changes.

## Order

After `flow-cancellation`. No following change.

## Impact

Text only: the archived `deep-failure-notices` design and tasks, and at most a doc comment in
`examples/src/dead_letters_upgrade.rs`. No API, wire format, schema, contract or guarantee changes.
