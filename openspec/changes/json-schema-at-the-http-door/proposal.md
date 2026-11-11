# Proposal

## Why

A send over HTTP is checked only by a node that hosts the target's type: it decodes the payload with serde and
refuses a bad one with `422`. A node that does not host the type accepts any payload, and the message fails later, in
its turn. So whether a bad payload is refused depends on which node the client reaches. Yet every node can read the
registered JSON Schema of every message type. ADR 0018 enforces the schema at the HTTP door.

## What Changes

- A send over HTTP is checked against the registered JSON Schema of its message type, on every node, whether or not
  the node hosts the type. A payload that does not match is refused with `400`.
- serde stays the judge at the turn.
- The schemas stay discoverable through `GET /types/{name}`.
- The test harness checks that a type's samples pass both the schema and serde.
- `docs/architecture.md` changes its row on validation: schemas are no longer "never used to validate".
- One new dependency, a JSON Schema validator, in `cicada-axum` only.

The simplest version: the check sits in the HTTP layer and nowhere else.

To decide in the design:

- Which validator crate, and which JSON Schema draft it must support.
- Whether a hosting node still decodes with serde at the door as well, and with which status (`422` today).
- Whether the state of `PUT /actors/{type}/{key}` is checked the same way. ADR 0018 names sends only.

## Capabilities

### New Capabilities

- `http-schema-check`: a send over HTTP must match the registered JSON Schema of its message type.

### Modified Capabilities

None.

## Out of scope

- Checking sends from code: the compiler and serde already do.
- Validating with the schema at the turn.
- Changing how schemas are generated or hashed.

## Order

Step 8 of ADR 0018. After `turn-observer`, before `cicada-app`.

## Impact

- `cicada-axum`, `cicada-test` (the sample checks), `cicada-client` (the new refusal), `docs/openapi.json`,
  `README.md`, `docs/architecture.md` (the validation row) and `CHANGELOG.md`.
- The wire format changes (a new `400`); ADR 0018 covers it.
