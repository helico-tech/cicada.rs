# 0026. The schema check at the HTTP door

Date: 2026-11-12

## Status

Accepted

## Context

ADR 0018 decides that a send over HTTP is checked against the registered JSON Schema of its message type, on every
node, and refused with `400` when it does not match. It leaves open what the change `json-schema-at-the-http-door`
had to settle: the state of a create, what a hosting node still does, and what the door does where a schema and
serde disagree. The change also keeps another status than ADR 0018 names. The project rule wants a change of the
wire format recorded.

## Decision

- **The refusal is `422`, not `400`.** This departs from ADR 0018. In this API `422` answers what a client sent that
  the system cannot take: an unknown message type, an invalid address, and until now a payload or a state that a
  hosting node could not decode. `400` answers a request that cannot be read. A payload that does not match its
  schema is of the first kind, and keeping `422` with the problem types `invalid-payload` and `invalid-state`
  breaks no client.
- **The state of a create is checked like the payload of a send.** A node that does not host a type stored any
  state until now, and an actor with a state that no node can decode fails every message.
- **A hosting node still decodes with serde,** after the schema check and with the same answer.
- **A repeated send is answered before it is checked,** when its message is pending or its outcome is still shown,
  as before.
- **The door is never stricter than the schema.** Formats are not asserted. A schema that the validator cannot use
  is not enforced; a warning is logged. The check loads nothing from outside the schema document.
- **The door can be looser than serde.** A node that does not host a type lets through what only serde refuses, and
  the message fails in its turn. serde stays the judge at the turn.
- **The validator and its test are in `cicada-axum`.** The core has no validator: it takes the check from its
  caller, and runs it where a hosting node decodes. The crate offers sample checks, so that a host can test that
  what its clients send passes both the schema and serde.
- **JSON that code sends is not checked.** `turn.send_raw` and `ActorSystem::send_message` take a payload as JSON,
  and only a hosting node's decoder checks it, as before.

## Consequences

- A payload or a state that serde accepts and the schema refuses is refused over HTTP from now on: a field under a
  `#[serde(alias)]`, a hand-written decoder that allows more than the schema says. The sample checks find such a
  type.
- A node that does not host a type answers `422` where it answered `202`.
- A type whose schema refuses what the type itself writes is refused at the door for every send. Making the schema
  right is a contract change. There is no switch; the sample checks find it before a deploy.
- A type whose schema the validator cannot use is not checked at the door. Its sample check fails.
- The core has two more calls and no new dependency.
