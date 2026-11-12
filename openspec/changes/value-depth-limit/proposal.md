# Proposal

## Why

A payload and a state are JSON, and JSON can nest. Four things go wrong with a deeply nested value, all older than
`json-schema-at-the-http-door`, whose validation found them:

- **JSON that code sends has no depth limit.** `turn.send_raw`, `ActorSystem::send_message` and a create take a
  value that code built, so the limit of a JSON parser (128 levels) does not apply. A value of some hundred to some
  thousand levels overflows a stack somewhere on its way (the limits, the store, the decoder, the executor), and
  the process ends without an error. Every turn it ran is then a lost claim.
- **The decoder of a hosting node can overflow a debug build.** For a type with large fields, a value that a
  request may carry (70 to 80 levels) already overflows the default stack of a thread of the runtime in a debug
  build, at the send or in the turn. A release build has room up to the 126 levels a request can carry.
- **A view can be deeper than a parser reads.** `GET /messages/{id}` wraps a payload two levels deeper. For a
  payload at the deepest level a request can carry, the answer is `200` and no JSON parser reads it, so
  `cicada-client` fails on it. The node is not harmed.

- **The decoder can take seconds for a small value.** For an untagged enum whose variants share a field, a value
  with one wrong leaf costs twice as much with every level: 24 levels, a request of 326 bytes, take about 10
  seconds on a node that hosts the type, at the send and again in the turn. Since `json-schema-at-the-http-door`
  a value that the schema refuses no longer reaches the decoder over HTTP; a value that the schema accepts and
  only the decoder refuses (a number that is too large) still does.

Over HTTP the schema check is safe since `json-schema-at-the-http-door`: it runs on a stack of its own.

Two more costs of a value were measured in the same validation. They are in proportion to the size of the body,
so they are no way to end a node with one request, but the factor is large:

- **Reading a body costs up to about 90 times its size.** A body of 1 MiB that is a list of 520,000 numbers takes
  about 90 MB as JSON values, before any check runs.
- **The decoder of a hosting node copies a payload before it decodes it,** about 17 times the body for a list of
  short items.

## What Changes

- **BREAKING** A payload and a state are at most 64 levels deep. A deeper value is refused like every other value
  beyond a limit: `OpError::Invalid` from outside a turn, `TurnError::Invalid` in a turn, and `400` over HTTP, before
  the schema check; a turn whose new state is too deep ends its message `failed`.
- **BREAKING** Over HTTP a payload or a state of 65 to 126 levels is `400`, where it was checked against its schema
  (`202` or `422`).
- An operator's retry checks the stored payload too: a payload that is too deep is `409 message-not-retryable`
  (a new `RetryResult::TooDeep`), and a bulk retry skips it.
- The check stops at the first level beyond the limit, so the check itself refuses a value of any depth with an
  error, and nothing after it meets a value deeper than 64 (a `Failed` that the runtime writes, 65). A `Value` built
  in code deep enough for serde_json's own drop to overflow is the host's.
- Every value accepted from this release on can be read back over HTTP: its views are at most 69 levels deep.
- Values stored before are delivered and decoded as before; a turn that writes such a state back fails.
- The error of a new state that a check refuses says `State cannot be stored:`.
- The ready-made actors do not change: a failed message whose payload is too deep for a dead letter fails the turn of
  the dead-letters actor and gets no letter.
- The core guide names the cost of serde's decoder for an untagged enum that holds itself, and the operations guide
  the memory that reading a body takes.

The simplest version: one limit, checked in the walk that already checks every payload and state.

## Capabilities

### New Capabilities

- `value-depth`: a payload and a state have a largest depth; a value within it is stored, viewed and decoded (for
  an ordinary type) without ending a node, and the check refuses a deeper one with an error.

### Modified Capabilities

- `dead-letters`: a failed message whose payload is too deep for a letter fails the turn that would record it.
- `http-schema-check`: the deepest value it checks is 64 levels, not 126, and a repeat beyond a limit is `400`.

## Out of scope

- The depth of a JSON Schema.
- Limits on the size of a value, which exist.

## Order

After `schema-reference-names`, the last of the line. Not a step of ADR 0018; the owner placed it on 2026-11-12.

## Impact

- `crates/cicada` (`runtime::limits`, the executor's error of a refused state, `runtime::system` with `retry_message`
  and `bulk_retry`, `RetryResult`, the docs of the calls that take JSON), `cicada-actors` (its tests and docs),
  `crates/cicada-axum` (the tests of deep values, of memory and of the schema check, which use 100 and 126 levels),
  the guides of the core ([docs/guide/core.md](../../../docs/guide/core.md)), of HTTP
  ([docs/guide/http.md](../../../docs/guide/http.md): the answer, how deep a view can be) and of the ready-made actors
  ([docs/guide/actors.md](../../../docs/guide/actors.md)), `docs/architecture.md` (the rows of a new state that cannot
  be stored in the table of outcomes and the table of causes, and the HTTP contract: the limits come before the
  repeated id and the schema check), `docs/operations.md` (memory per body, the upgrade), `CHANGELOG.md`, and
  ADR 0029 with its row in `docs/adr/README.md`.
- Hosts: Chorus keeps the output of a task three levels into the state of a workflow; a task output deeper than 61
  levels fails the workflow's turn after the upgrade. Karakuri does not use cicada.
- The HTTP contract gains a refusal (`400`) where a value of 65 to 126 levels was checked against its schema: ADR 0029.
