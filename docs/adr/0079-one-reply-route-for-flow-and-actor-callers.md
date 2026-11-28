# 0079. One reply route for flow and actor callers

Date: 2026-11-28

## Status

Accepted for reply-route. Adds to [ADR 0044](0044-flow-requests-waits-and-cancellation.md), which stays in force.

## Context

A target of `step.request` replies through a `ReplyTo<T>`: the run's address and the request's message id (ADR 0044).
An actor that calls the same target has only its own address. A port called by both needs two messages, or each
project writes its own enum with its own wire and untested send. Factory has such ports (`repos.mirror`).

## Decision

- `cicada-actors` has `ReplyRoute<T>`: `Flow(ReplyTo<T>)` or `Actor { to, correlation }`, with one `send`. The flow
  form sends as `ReplyTo::send`. The actor form sends the value, a message of the port's reply family, to `to`, with
  the route's correlation id when it has one and else the turn's, no retry policy and the ordinary headers and
  retention.
- The flow form's wire is `ReplyTo`'s, `{"run", "request"}`, so a saved `ReplyTo<T>` decodes as the flow form. The
  actor form is `{"actor"}` with an optional `"correlation"`. The key decides: an object with `actor` is the actor form, one with `actor` and `run` or
  `request` does not decode. The schema is the same for every `T`.
- The correlation id travels in the route, so a target that replies in a later turn still returns the caller's id. An
  actor caller matches replies by it; the route carries no request id.
- That the caller's family holds the reply family's variants is checked at runtime only: a missing type name fails the
  target's send, as `send_to` does, and wrong fields show when the caller decodes the reply.

## Consequences

- A port declares one field `reply: ReplyRoute<T>`; flows write `r.into()`, actors `ReplyRoute::actor(address).correlation(id)`.
- `ReplyTo<T>` and its wire are unchanged. A port can move a `ReplyTo<T>` field to `ReplyRoute<T>` without changing
  saved messages, but its contract hash changes.
- Tests of a port encode each reply variant and decode it as each caller's family.
