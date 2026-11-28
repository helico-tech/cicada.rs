# Proposal

## Why

Consumers want to build and test one actor without its real neighbours: dependency inversion (ports and adapters)
between actors. Cicada already splits the two parts. An `ActorRef` with its message family is the contract (the
port); any `impl Actor` with that `Ref` is an implementation (the adapter); `host(..)` on `App` or `TestActors`
chooses which one runs. But no guide or example says so, and the examples couple both ways: the saga's
`InventoryMsg::Reserve { reply_to: Order }` makes inventory import the order type. So a consumer cannot see how to
test an order with a fake inventory. The owner picked option A on 2026-11-28: a port module with a real and a fake
actor for the same ref.

## What Changes

The request/reply example already is a port: `examples/src/reply/pricing.rs` holds the `Pricing` ref, its request
family and the reply family it owns, and the pricing actor replies to `message().from` with `send_to`, so it imports
nothing of the cart. What is missing is the other half: a test of the real cart against fakes. So this change adds
tests to `examples/tests/reply.rs`, not a new example (decided 2026-11-28 under the owner's A1, as the simplest way to
show it; the proposal's earlier `ports.rs` would repeat the reply example):

- Two fake pricing actors in the test file, each an `impl Actor` with `type Ref = Pricing`: one that always quotes a
  fixed price, one that always refuses. Tests on virtual time host the real `CartActor` with one fake each and check
  the cart's lines.
- One test that each variant of `PricingReply` decodes as a `CartMsg` (`cicada::message::{encode, decode}`), since
  that match is checked only at runtime. A plain test; a helper in `cicada-test` waits for `test-probe`.
- A docs handoff for a section of `docs/guide/design.md` on ports and fakes: the port module owns the ref, the
  request family and the reply family; replies go by address; a fake is another `impl Actor` for the same ref, hosted
  instead of the real one; trait injection for outside systems as with `ContainerRuntime`; one implementation per
  type name per node; a fake's contract hash differs, so it belongs in a test store, not a shared one.

An explicit reply address in the request (for a caller that is not the sender, such as a flow) is `reply-route`.

## Out of scope

- A new example module, and rewriting the saga or other examples to the port style.
- A generic probe actor in `cicada-test` that records messages and sends scripted replies: a later change if wanted.
- Changes to the runtime or the public API: none are needed.
- Adapter actors that translate between two protocols (option C) and target addresses chosen at runtime (option B,
  beyond reply addresses).

## Lane

Lane: fast. Tests only, plus a docs handoff; no public API, wire, schema, contract or guarantee change.

## Order

After `container-network-isolation`. Before `reply-route`.

## Impact

Crates: cicada-examples.

`examples/tests/reply.rs` only. No public API, wire, schema or contract change. Docs handoff: the design guide section, and an actor-design skill route if the new heading needs one.
