# Proposal

## Why

A port is often called by both kinds of caller. In Factory, `repos.mirror` gets `Ensure` from an actor
(`agents.session`) and from flows (`security.scan`, `delivery.revision`). A flow caller needs a `ReplyTo<T>` from
`step.request`; an actor caller needs an address and a reply message. Today the port must offer two messages, or each
project writes its own enum of the two with its own wire shape and untested send. The owner agreed on 2026-11-28 (P1)
to give Cicada one reply route for both.

## What Changes

- `cicada-actors` gains `ReplyRoute<T>` (the design's name for `Reply<T>`): `Flow(ReplyTo<T>)` or `Actor { to }`, with one `send(turn, &value)`. For a flow
  it sends `FlowMsg::Reply` as `ReplyTo::send` does; for an actor it sends `value` as a message to `to`, so `T` is the
  port's own reply family and the caller's family holds the same variants.
- A flow caller writes `|r| Ensure { repo, reply: r.into() }`; an actor caller writes `reply: ReplyRoute::actor(address)` (with `.correlation(id)`).
- The wire of the flow form is the wire of `ReplyTo` (`{"run", "request"}`), so a saved `ReplyTo<T>` decodes as
  `ReplyRoute::Flow`; the actor form is `{"actor": "<address>"}`.

To decide in the design: the name (`Reply`, `ReplyRoute`); whether the actor form also carries the request's message
id for callers with several requests open (or the port's reply carries a business key); how the actor send is typed
(the reply family's tag must match a variant of the caller's family, checked only at runtime) and what a test helper
for that match looks like, shared with `actor-ports`; correlation id and headers of the reply (as `ReplyTo::send`).

## Out of scope

- A flow as the target of a request: `flow-request-target`, which then takes this route in its `Start`.
- Compile-time checks that a caller's family can decode the reply.
- Changing `ReplyTo<T>` or its wire.

## Order

After `actor-ports`. Before `flow-request-target`.

## Impact

Crates: cicada-actors.

`crates/cicada-actors/src/flow.rs` (or a module of its own) and its tests; the schema of `ReplyRoute<T>` in any contract
that uses it. A new public type and ADR 0079; no change to existing wire, schema or contracts. Docs handoff: the design guide's
section on ports (from `actor-ports`), the actors guide's requests text.
