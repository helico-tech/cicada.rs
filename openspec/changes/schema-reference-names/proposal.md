# Proposal

## Why

A message schema keeps only the definitions it refers to. To find them, `schema.rs` takes the name out of each
`$ref` and looks it up in `$defs`. A `$ref` is a JSON Pointer inside an address, so a name with a space, a `/`, a
`~` or `<>` is written in an escaped form there, while the key in `$defs` holds the name as it is. The two do not
match, and the definition is left out of the message schema.

Until 0.12.0 nobody noticed: nothing read a schema. Since `json-schema-at-the-http-door` the door checks against
it. A schema with a reference to a definition that is not there cannot be used, so the door does not enforce it,
and `assert_message_sample` fails for the type with "cannot be used". A type gets such a name through
`#[schemars(rename = "…")]`. Found when `json-schema-at-the-http-door` was validated.

## What Changes

- The name of a `$ref` is read as a JSON Pointer in an address: unescaped before it is looked up in `$defs`. The
  same holds where a newtype variant is replaced by the definition it refers to.
- **BREAKING** for a type that is hit: its message schemas gain the definitions they lacked, so its contract hash
  changes, and the node follows "Changing a contract" in the operations guide. No other type changes.

The simplest version: unescape in the two places, and a test with a renamed type.

To decide in the design:

- Whether a node should refuse to start for a type whose schema refers to a definition that is not there, so that a
  later fault of this kind is found at the start and not at the door.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `type-registry`: a message schema holds every definition it refers to, whatever its name.

## Out of scope

- Changing the names that schemars gives to definitions.
- Any other change to how schemas are generated or hashed.

## Order

After `docs-per-layer`. It is not a step of ADR 0018; it waits for the owner to place it.

## Impact

- `crates/cicada` (`schema`), the contract snapshots of a type that is hit (none in this repository), `CHANGELOG.md`.
- A contract hash can change, for a type with such a name only.
