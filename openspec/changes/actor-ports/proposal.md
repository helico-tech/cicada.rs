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

- A new worked example in `examples/` (working name `ports.rs`). A port module holds the `Inventory` ref, its request
  family and a reply family it owns. The request carries `reply_to: ActorAddress`; the callee answers with
  `turn.send_to(..)`, so it imports nothing of its callers. A real inventory actor and an order actor use only the
  port.
- Its tests on virtual time host the real order actor with a fake inventory for the same ref (one that always
  reserves, one that refuses), and one test that each reply of the port decodes as the order's message family, since
  that match is checked only at runtime.
- A docs handoff for a new section of `docs/guide/design.md` (ports and fakes: where the port lives, replies by
  address, trait injection for outside systems as with `ContainerRuntime`, one implementation per type name per node,
  a fake's contract hash in a shared store) and the example's row in the core guide's table.

To decide in the design: the example's name and domain (a new one, or a port version of the saga); whether the reply
family check is a plain test with `cicada::message::{encode, decode}` or worth a helper in `cicada-test`.

## Out of scope

- A generic probe actor in `cicada-test` that records messages and sends scripted replies: a later change if wanted.
- Changes to the runtime or the public API: none are needed.
- Rewriting the saga or other examples to the port style.
- Adapter actors that translate between two protocols (option C) and target addresses chosen at runtime (option B,
  beyond reply addresses).

## Order

After `worker-view`. Before `reply-route`.

## Impact

Crates: cicada-examples.

`examples/src/ports.rs` (or its folder), its row in `examples/src/lib.rs` and `examples/tests/ports.rs`. No public API,
wire, schema or contract change. Docs handoff: the design guide section, the core guide's example table, and an
actor-design skill route if the new heading needs one.
