# Docs handoff for actor-ports

Version 0.26.0. Refresh the dependency pins in `docs/how-to/install.md` once it is published.

## What changed

The [request/reply tests](../../../examples/tests/reply.rs) show how to keep the real `CartActor` while replacing its
pricing neighbour. `the_cart_prices_lines_with_a_fixed_pricing_fake` checks two priced lines.
`the_cart_records_a_rejection_from_a_pricing_fake` checks a refused line. Both tests run on virtual time.
`pricing_replies_decode_as_cart_messages` checks that both of the pricing port's replies decode as cart messages.

## 1. Ports and fakes

**Where.** Add a subsection titled “Ports and fakes” under “Design protocols” in `docs/guide/design.md`.

**Text.** A port module owns the `ActorRef`, its request family and its reply family, as
[`pricing.rs`](../../../examples/src/reply/pricing.rs) does with `Pricing`, `PricingMsg` and `PricingReply`. The
pricing actor sends a reply to the address in `message().from` with `send_to`, carrying the request's correlation
id. This lets the pricing actor serve callers without importing their types. Each caller accepts the reply names
and fields in its own message family. The wire match is checked at runtime, so test it with `message::encode`
and `message::decode`.

A fake is another `impl Actor` with the same `Ref`. Choose the implementation with `App::host` or through the
node builder in `TestActors::builder().with(...)`. The [cart tests](../../../examples/tests/reply.rs) host the real
cart with either `FixedPricing` or `RejectingPricing`, then check its state. When the dependency is an outside
system, pass a trait implementation into the actor instead. `ContainerRuntime` and `FakeRuntime` are examples.

**Caveats.** Host one implementation per type name on a node. The contract hash comes from the type name, the
message family and the state schema. A fake with a different state type gets another hash, and one with the same
state type gets the same hash while behaving differently, so a fake belongs in a test store, never in a shared one.

## 2. Actor-design skill route

**Where.** `skills/actor-design/SKILL.md`, “Source routes”.

**Text.** If the new subsection needs a direct route, add `docs/guide/design.md#ports-and-fakes` to the messages
and replies row when the heading exists.

## Blocking

None.

## Status

The reply tests and docs checks passed (6 and 11 tests). Formatting, Clippy, strict OpenSpec validation and the full
gate passed, including the PostgreSQL-enabled workspace suite (4,107 tests). The guide subsection and any
accompanying skill route are proposed for the docs session.
