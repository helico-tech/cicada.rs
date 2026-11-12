# Proposal

## Why

A payload and a state are JSON, and JSON can nest. Three things go wrong with a deeply nested value, all older than
`json-schema-at-the-http-door`, whose validation found them:

- **JSON that code sends has no depth limit.** `turn.send_raw`, `ActorSystem::send_message` and a create take a
  value that code built, so the limit of a JSON parser (128 levels) does not apply. A value of some hundred to some
  thousand levels overflows a stack somewhere on its way (the limits, the store, the decoder, the executor), and
  the process ends without an error. Every turn it ran is then a lost claim.
- **The decoder of a hosting node can overflow a debug build.** For a type with large fields, a value that a
  request may carry (70 to 80 levels) already overflows the default stack of a thread of the runtime in a debug
  build, at the send or in the turn. A release build has room up to the 126 levels a request can carry.
- **A view can be deeper than a parser reads.** `GET /messages/{id}` wraps a payload three levels deeper. For a
  payload at the deepest level a request can carry, the answer is `200` and no JSON parser reads it, so
  `cicada-client` fails on it. The node is not harmed.

Over HTTP the schema check is safe since `json-schema-at-the-http-door`: it runs on a stack of its own.

Two more costs of a value were measured in the same validation. They are in proportion to the size of the body,
so they are no way to end a node with one request, but the factor is large:

- **Reading a body costs up to about 90 times its size.** A body of 1 MiB that is a list of 520,000 numbers takes
  about 90 MB as JSON values, before any check runs.
- **The decoder of a hosting node copies a payload before it decodes it,** about 17 times the body for a list of
  short items.

## What Changes

- **BREAKING** A payload and a state may be nested only so deep. A deeper value is refused like every other value
  beyond a limit: `OpError::Invalid` from outside a turn, `TurnError::Invalid` in a turn, and `400` over HTTP.
- The limit leaves room for the levels a view adds, so every value that is accepted can be read back.
- A decode of a value at the limit does not overflow a stack, in a debug or a release build, at the send and in
  the turn.

The simplest version: one more check in `runtime::limits`, next to the size of a value.

To decide in the design:

- The limit. It must be below what a parser reads less the levels of the deepest view, and low enough for a
  decoder in a debug build, or the decode gets a stack of its own as the schema check has.
- Whether values that are stored today and are deeper than the limit need anything.
- Whether the two costs in proportion to the body need anything: a limit on the number of values in a body, a
  decoder that does not copy, or only a line in the operations guide on the size of a body.

## Capabilities

### New Capabilities

- `value-depth`: a payload and a state have a largest depth, and a value within it never ends a node.

### Modified Capabilities

None.

## Out of scope

- The depth of a JSON Schema.
- Limits on the size of a value, which exist.

## Order

After `schema-reference-names`. It is not a step of ADR 0018; it waits for the owner to place it.

## Impact

- `crates/cicada` (`runtime::limits`, the docs of the calls that take JSON), `cicada-axum` (the answer),
  `README.md`, `docs/architecture.md` (the limits) and `CHANGELOG.md`.
- The wire format gains a refusal; the design says whether that needs an ADR.
