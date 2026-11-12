# Design

Described on 2026-11-12 (change loop, Describe), while `docs-per-layer` is built. The owner placed this change after
`docs-per-layer` ("Yeah, do them") under the go-ahead of 2026-11-11. Every decision below is made by the main session;
each gives the options and why. Reviewed twice; what changed after each review is listed at the end.

## Context

What the code does today (release 0.14.0):

- `schema::message_schemas` cuts the schema of a message family into one document per message type, and keeps in
  each only the `$defs` it refers to. It finds them by taking the text after `#/$defs/` in each `$ref` and looking it
  up in `$defs` as it is. The same happens where a newtype variant is replaced by the definition it refers to.
- schemars writes a `$ref` as a JSON Pointer in the fragment of an address: `~` becomes `~0`, `/` becomes `~1`, and
  every byte outside a small set (letters, digits, `_`, `-`, `.`, `!`, `$`, `&` to `;`, `=`, `?`, `@`) is
  percent-encoded, among them a space, `<`, `>`, `[`, `]`, `{`, `}`, `|` and every byte of a letter that is not
  ASCII. The key in `$defs` holds the name as it is. The validator of the door reads such a reference as RFC 6901
  says: it percent-decodes, splits at `/`, and unescapes each segment.
- A type gets a name that needs escaping through `#[schemars(rename = "…")]`, through a container
  `#[serde(rename = "…")]` (schemars takes the name that serde decodes), or as a Rust type whose name has a letter
  that is not ASCII (`Größe`).
- **A field of such a type** loses its definition from the message schema. That schema refers to a definition it
  does not hold. The door cannot use it, so it does not check that message type, and warns once (ADR 0026);
  `assert_message_sample` fails with "cannot be used". The contract was registered so, and a node starts.
- **A newtype variant of such a type** fails the lookup, and `message_schemas` answers that the family "must be an
  enum with #[serde(tag)]". The node does not start. No contract was ever registered for such a family.
- **A family that holds itself** (a message `Batch { items: Vec<Msg> }`) gets `"$ref": "#"` from schemars: a
  reference to the root of the family's schema. After the cut, `#` is the root of the one message's document, so the
  door checks each nested message against the schema of `Batch` and refuses a valid payload. The type itself
  decodes it.
- No type of this repository, and none of Chorus, is hit by any of the three (a search for such renames, for type
  names with letters that are not ASCII, and for families that hold themselves). Karakuri does not use cicada yet.
- The door may be looser than the decoder, never stricter (ADR 0026): a node that hosts a type decodes what the
  schema accepted, and a node that does not lets it through to fail in its turn.

## Goals / Non-Goals

**Goals**

- A message schema holds every definition it refers to, whatever its name.
- The door never refuses a valid payload of a family that holds itself.
- A type that is not hit keeps its contract byte for byte.

**Non-Goals**

- Changing the names that schemars gives to definitions, or any other part of how schemas are made or hashed.
- A check at the start of a node for schemas that refer to something they do not hold (decision 4).

## Decisions

### 1. A reference is read as RFC 6901 says, and the name is its first segment

`schema.rs` reads a `$ref` that starts with `#/$defs/` as a JSON Pointer in a fragment: it percent-decodes the rest,
splits it at `/`, turns `~1` into `/` and `~0` into `~` in each segment, and takes the first segment as the name of
the definition. A pointer of several segments (`#/$defs/Line/properties/sku`) needs the definition `Line`. That is
how the validator reads it. Both places that read a reference use one function.

A rest that does not percent-decode (a `%` that is not followed by two hex digits, bytes that are not UTF-8) is
split and unescaped as it is.

Not chosen: a function of schemars. Its percent-decoding is in a private module; `Schema::pointer` is public but
looks up a value in a schemars `Schema`, where we hold `serde_json` values and need the name, not the value. A small
function of our own, tested against what schemars writes, is simpler.

### 2. A newtype variant whose definition is missing says so

When the definition that a newtype variant refers to is not there after decoding, `message_schemas` answers that
the variant refers to a definition the family's schema does not hold, and names it, instead of "must be an enum with
#[serde(tag)]". After decision 1 that happens only for a hand-written schema.

### 3. A reference to the family accepts any message

When a family holds itself, schemars refers to the family with `#`. In each message document that holds such a
reference, the reference becomes the schema `true`, which accepts any value. The nested messages are then checked
by the decoder: at the door of a node that hosts the type, and in the turn everywhere. A document without such a
reference does not change.

- **`true` (chosen, after the second review).** It ends the false refusal with a few lines, adds no name and no
  definition to a contract, and keeps the door as ADR 0026 allows it to be: looser than the decoder, never
  stricter. Only the documents that hold `#` change.
- The whole family as one more definition of each such document, under a reserved name (the second version of
  this design). The door would check the nested messages too, but the definition would have to be cut out of the
  family's own document, with every definition it refers to copied up; the reserved name could clash with a renamed
  type; contracts would grow by a copy of the family; and the door would meet a shape (a `oneOf` of every message at
  each level) that was never measured against its limits. All of that for a kind of family that no known host has.

The rewrite walks only the keywords that hold schemas, as `contract::strip_descriptions` does, so a `#` in a
`const`, an `enum`, a `default` or in `examples` is data and stays.

### 4. No check at the start of a node

The first version of this design let a node refuse to start when a schema refers to a local definition it does not
hold. Not chosen, after the review:

- After decisions 1 to 3, what is left is a mistake in a hand-written schema, of the same kind as a pattern the
  validator cannot compile, which the door also only warns about (ADR 0026). Refusing one and warning about the other
  would draw a line nobody can explain.
- It would add a way for an upgrade to stop a node, also for a type this bug never touched.
- The sample checks already fail for such a schema ("cannot be used").

### 5. The contract of a type that is hit changes

- A field of a type with an escaped name: the message schema gains the definition it lacked.
- A family that holds itself: the message documents that refer to `#` get `true` there.
- A hand-written schema with a reference of several segments (`#/$defs/Line/properties/sku`): its message schema
  now keeps `Line`, which it lost before.
- A newtype variant of a type with an escaped name: the type could not start before; it can now, with its first
  contract.

The contract hash of these changes, so a node of 0.15.0 refuses to start against the registration of the
release before (`ContractConflict`). A type that is not hit keeps its contract byte for byte; the contract snapshots
of this repository do not change, and a test says so.

No ADR: the format of a contract does not change, only faults in what it held.

## Risks / Trade-offs

- **A type that is hit costs a stop of the world, or a new type name.** "Changing a contract" means stopping every
  node that hosts it, deleting every actor of the type (`DELETE /types` answers `409` while one exists) and removing
  its registration, or moving to a versioned type name. The changelog says how to tell: the host's contract snapshot
  of the type changes, or a node refuses to start with a `ContractConflict`.
- A hand-written schema that refers to a definition it does not hold is still not enforced at the door, and only
  warned about (decision 4).

## Migration Plan

Version 0.15.0. No migration of the store.

1. After the dependency is raised to 0.15.0 and before any node is deployed, run the contract snapshot tests of every
   type (`assert_contracts_match`) against the snapshots of 0.14.0. A snapshot that changes names a type that is hit.
2. For such a type, follow "Changing a contract" in the operations guide: a versioned type name, or a stop of the
   world that deletes its actors.

## Changed by the review

The first version read everything after `#/$defs/` as one name, missed the container `#[serde(rename)]` as a cause,
described the newtype case wrongly (it could not start, so nothing breaks for it), refused to start a node for a
schema with a missing local definition, and gave a wrong reason for not using schemars. The review also found the
family that holds itself; decision 3 fixes it in this change, because it is the same function and the same kind of
fault.

The second review found that the first answer to that family (its schema as a definition `$family` in every
message document) did not say what that definition holds, would have broken the documents that refer to its inner
definitions, would have changed the documents of `Failed` and `Stopped` for a watcher whose family holds itself,
could clash with a renamed type, and put a new shape in front of the door. Decision 3 now replaces `#` with `true`
in the documents that hold it. The Impact no longer names a start check, and the migration says when to run the
snapshots.

## Open Questions

None.
