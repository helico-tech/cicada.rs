# Proposal

## Why

A send over HTTP is checked only by a node that hosts the target's type: it decodes the payload with serde and
refuses a bad one with `422`. A node that does not host the type accepts any payload, and the message fails later, in
its turn. So whether a bad payload is refused depends on which node the client reaches. The same holds for the state
of a create, and there the result is worse: an actor whose state no node can decode fails every message. Yet every
node can read the registered JSON Schemas of every type. ADR 0018 enforces the schema at the HTTP door.

## What Changes

- A send over HTTP is checked against the registered JSON Schema of its message type, on every node, whether or not
  the node hosts the type. A payload that does not match is refused with `422 invalid-payload`.
- The state of a create over HTTP is checked against the registered state schema in the same way, and refused with
  `422 invalid-state`.
- The status stays `422`, as for a payload that does not decode today. ADR 0018 says `400`; ADR 0026 records why
  this change keeps `422`.
- **BREAKING** A payload or a state that serde accepts and the schema refuses is refused from now on, by every
  node: a field under a `#[serde(alias)]`, a type whose hand-written decoder allows more than its schema. And a
  node that does not host a type refuses what it accepted. A type whose schema refuses what the type itself writes
  must get a right schema, which is a contract change.
- A node that hosts the type still decodes with serde what the schema accepts. A refusal by the schema names no place
  and runs nothing else. serde stays the judge at the turn.
- A send that repeats an id is answered as a repeat before it is checked, as today.
- Formats are not asserted, and a schema that the validator cannot use is not enforced.
- The schemas stay discoverable through `GET /types/{name}`.
- `cicada-axum` offers `assert_message_sample` and `assert_state_sample`: a sample must pass both the schema and
  serde. The repository's own types get such checks.
- `ActorSystem::send_message_checked` and `create_actor_checked` take the check from the caller; the core has no
  validator.
- `docs/architecture.md` changes its row on validation: schemas are no longer "never used to validate".
- Two new dependencies, both of `cicada-axum` only: the JSON Schema validator `jsonschema` without its default
  features, and `peak_alloc`, a dev-dependency for the test of the memory a refusal holds (`tests/memory.rs`).

The simplest version: the validator sits in the HTTP layer and nowhere else.

## Capabilities

### New Capabilities

- `http-schema-check`: a send and a create over HTTP must match the registered JSON Schema.

### Modified Capabilities

- `undecodable-messages`: a payload is refused at the door by the schema on every node, and by the decoder on a
  hosting node.

## Out of scope

- Checking sends and creates from code. The compiler and serde check typed ones; JSON that code sends
  (`turn.send_raw`, `ActorSystem::send_message`) stays unchecked, as today.
- Validating with the schema at the turn.
- Changing how schemas are generated or hashed.

## Order

Step 8 of ADR 0018. After `turn-observer`, before `cicada-app`.

## Impact

- `cicada-axum` (the check, the sample checks, the validator), `cicada` (the two checked calls, the docs of
  `schema`), `cicada-client` and `cicada-cli` (the wording of the refusals), the examples and `cicada-extras`
  (sample checks), `e2e` (`cluster_api.rs` pins that a node that only sends accepts a bad payload),
  `docs/openapi.json`, `README.md`, `docs/architecture.md` (the validation row, the HTTP contract),
  `docs/operations.md` (the upgrade note, the warning) and `CHANGELOG.md`.
- The wire format: no status changes; a node that does not host a type answers `422` where it answered `202`.
  ADR 0018 covers the check of a send; ADR 0026 records what this change adds, and the status it keeps.
- Hosts: Chorus and Karakuri add the sample checks for the types their clients reach over HTTP.
