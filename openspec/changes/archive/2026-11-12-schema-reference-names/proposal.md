# Proposal

## Why

A message schema keeps only the definitions it refers to. To find them, `schema.rs` takes the name out of each
`$ref` and looks it up in `$defs`. A `$ref` is a JSON Pointer inside an address, so a name with a space, a `/`, a
`~`, `<>` or a letter that is not ASCII is written in an escaped form there, while the key in `$defs` holds the name
as it is. The two do not match, and the definition is left out of the message schema. A type gets such a name
through `#[schemars(rename)]`, a container `#[serde(rename)]`, or its Rust name.

Until 0.12.0 nobody noticed: nothing read a schema. Since `json-schema-at-the-http-door` the door checks against
it. A schema with a reference to a definition that is not there cannot be used, so the door does not check that
message type, and `assert_message_sample` fails for it with "cannot be used". Found when
`json-schema-at-the-http-door` was validated. The review of this change found a second fault of the same function:
a family that holds itself refers to itself with `#`, which after the cut means the one message, so the door refuses
valid payloads of such a family.

## What Changes

- A reference is read as a JSON Pointer in a fragment, as RFC 6901 says: percent-decoded, split into segments,
  unescaped. The name of the definition is its first segment. The same holds where a newtype variant is replaced by
  the definition it refers to, and a missing definition there is named in the error.
- A reference to the whole family (`#`) accepts any value in the schema of a message that holds it, so the door
  never refuses a valid nested message of a family that holds itself; the decoder checks it.
- **BREAKING** for a type that is hit: its message schemas change, so its contract hash changes, and the node follows
  "Changing a contract" in the operations guide. No other type changes.

The simplest version: one function that reads a reference, `true` for a reference to the family, and tests with
each kind of type that is hit.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `type-registry`: a message schema holds every definition it refers to, whatever its name.

## Out of scope

- Changing the names that schemars gives to definitions.
- Any other change to how schemas are generated or hashed.

## Order

After `docs-per-layer`, before `value-depth-limit`. Not a step of ADR 0018; the owner placed it on 2026-11-12.

## Impact

- `crates/cicada` (`schema`), the contract snapshots of a type that is hit (none in this repository), the docs of the
  contract in the core guide, `docs/operations.md` (the upgrade), `CHANGELOG.md`.
- A contract hash can change, for a type with such a name only. No ADR: the format of a contract does not change.
