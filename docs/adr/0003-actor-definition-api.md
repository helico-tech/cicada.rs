# 0003. Actor definitions: a trait, a tagged serde enum and a required unhandled policy

Date: 2026-10-07

## Status

Accepted. ADR 0018 removes the required `UNHANDLED` and the unhandled policy: a message that cannot be decoded fails.
The message type is declared once, on the ref.

## Context

Kotlin defines a type with a builder DSL: `on<T> { }` per message class, checked at node start, and a mandatory
`unhandled { }`. Rust has no reified generics or sealed-class reflection, but it has exhaustive `match` and associated
consts.

## Decision

- A ref is a serde struct implementing `ActorRef` (`const TEMPLATE`, `type Message`). Its fields are the template
  parameters, converted with a small serde serializer/deserializer (scalars as text). Field names are checked against
  the template at node start.
- A message family is an enum with `#[serde(tag = "type")]`. On the wire a message is its type name (the tag) plus the
  payload (the variant's fields without the tag), exactly the Kotlin format. Per-message JSON Schemas are split out of
  the enum's schema.
- An actor type implements `Actor`: associated `Ref`, `State` (or `Stateless`), `Message`, a required
  `const UNHANDLED: Unhandled` (Fail, Drop or Custom) and one `async fn handle` that matches on the message. The
  compiler checks handler coverage and the presence of the unhandled policy.
- Handlers return `Result<(), HandlerError>`; a permanent failure is `HandlerError::permanent`. Panics are caught and
  treated as failures.
- No proc macro. One can be added later as optional sugar.

## Consequences

- Checks that Kotlin does at node start (every message handled, unhandled declared) move to compile time.
- A variant field named `type` would clash with the tag; serde rejects such payloads.
