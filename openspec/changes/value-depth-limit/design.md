# Design

Described on 2026-11-12 (change loop, Describe), while `schema-reference-names` is built. The owner placed this change
after `schema-reference-names` ("Yeah, do them") under the go-ahead of 2026-11-11. Every decision below is made by
the main session; each gives the options and why. Reviewed twice; what changed after each review is listed at the
end. ADR 0029 records the rule.

## Context

What the code does at 0.15.0 (`schema-reference-names` built on 0.14.1):

- `runtime::limits::check_json` checks a payload or a state for a NUL character, by a walk that calls itself for
  each level of the value. It runs for a send and a create from outside a turn, for a send and a create in a turn,
  and for the new state of a turn. Nothing checks the depth of a value.
- Over HTTP the body is read with the default limit of serde_json, 128 levels, so a payload or a state can have at
  most 126. Code can build any depth. A value of some thousand levels overflows the stack of a thread of the runtime
  in a walk that calls itself (`contains_nul`, serde's encoder and decoder, the store's encoding of an envelope), and
  the process ends without an error; every turn it ran is then a lost claim.
- The decoder of a type needs stack per level of a value; how much depends on the type. The validation of
  `json-schema-at-the-http-door` measured up to 676 KiB at 126 levels in a debug build for eleven ordinary shapes
  (about 5 KiB a level), and 28.5 KiB a level for a type with six inline arrays of 32 numbers. A thread of the runtime
  has 2 MiB.
- A view wraps a value: `GET /messages/{id}` puts a payload two levels deeper, a page of `GET /outcomes` four, and
  `GET /actors/{type}/{key}` a state one. For a payload of 126 levels a view is deeper than a parser reads.
- A `Failed` notification holds the payload of the failed message one level down; the runtime writes it. A
  notification that fails tells nobody, so a `Failed` is never about a `Failed`, but an operator's retry
  (`ActorSystem::retry_message`) writes the stored payload again as an ordinary message without any check, and a
  retried `Failed` that fails is then held one level deeper again.
- The ready-made actors hold values deeper: a letter of dead letters holds the payload of a failed message three
  levels into the state; a `Publish` and a `Deliver` of a topic hold a payload two levels into theirs; a child entry
  of a supervisor holds the child's state three levels into its state, and the payload of its start message four.
- serde's decoder of an untagged enum that holds itself tries each variant at each level: for a value with one wrong
  leaf the time doubles with every level (24 levels, 326 bytes, about 10 seconds). Since
  `json-schema-at-the-http-door` a value that the schema refuses does not reach the decoder over HTTP; since
  `schema-reference-names` the door does not check below a nested message of a family that holds itself.
- Reading a body takes up to about 90 times its size as JSON values (a list of 520,000 numbers in 1 MiB), and the
  decoder copies a payload into a new object to put the tag of its message type back, which an internally tagged
  enum needs.
- The spec `http-schema-check` promises that a value of 126 levels is checked against its schema (`202` or `422`),
  and `crates/cicada-axum/tests/deep_values.rs` (126 levels), `memory.rs` (100 levels) and `schema_check.rs` test it.
- Chorus keeps free-form JSON (`Value`) as the output of a task: a `StepDone` holds it one level down, and the state
  of a workflow three (`steps`, a list of steps). Karakuri does not use cicada.

## Goals / Non-Goals

**Goals**

- No value that a node accepts can end the process by its depth: not in a check, not in the store, not in a view,
  not in the decoder of an ordinary type.
- Every value that a node accepts from this release on can be read back over HTTP.

**Non-Goals**

- Bounding the time of serde's decoder for an untagged enum that holds itself (decision 5).
- A limit on the number of values in a body, or a decoder that does not copy (decision 6).
- Guarding a JSON value built in code that is too deep for serde_json's own drop (decision 2).
- The depth of a JSON Schema.

## Decisions

### 1. A payload and a state are at most 64 levels deep

`runtime::limits::MAX_DEPTH` is 64: a value holds at most 64 arrays and objects inside each other. `{"a": {"b": 1}}`
is 2 deep; a scalar is 0.

- **64 (chosen).** Real messages are a few levels deep; a tree that a host keeps in a message (an expression, a
  plan) has room. A view of a value at the limit is at most 69 levels deep (a `Failed` adds one, a page of outcomes
  four), which every parser that allows 128 reads. The decoders that were measured need at most 0.4 MiB in a debug
  build for 64 levels of an ordinary type, and 1.8 MiB for the type with large inline arrays, which is close to the
  2 MiB of a thread (Risks). A release build needs a fraction of that.
- 59, so that every view fits the default of 64 of System.Text.Json and Json.NET. No client of .NET is known, and
  both raise their limit with one setting; the HTTP guide says how deep a view can be.
- 32. More room on the stack, but a host's tree type reaches it, and the ready-made actors (which add up to four
  levels) would leave 28.
- About 122, the most at which every view still parses (a view adds up to five levels, a parser reads 128). The
  smallest break: only 123 to 126 levels would be refused over HTTP, and Chorus would keep 118. But the decoder of a
  type with large fields overflows a debug build at 70 to 80 levels, and a debug build is where hosts run their tests;
  and nothing would be left for a wrapping that a later change adds. The case for 64 rests on that margin.
- 126, the most a body can carry. A view would no longer parse, and the decoder of a large type overflows a debug
  build well before it.

A setting for the limit was not chosen: one number keeps every node of a system the same, and nobody has asked for
another.

### 2. The check stops at the first level beyond the limit

`check_json` finds a NUL character and measures the depth in one walk, and stops at the first level beyond the limit.
It calls itself for each level, but never more than 65 times, so the check refuses a value of any depth with an
error, and nothing that comes after it (the encoder, the store, the decoder, a view) meets a value deeper than 64,
except a `Failed` that the runtime writes (65). The check runs where it runs today; no answer changes for a value
within the limit.

A value that is too deep is refused as the other limits refuse:

| Where | Answer |
| --- | --- |
| a send or a create from outside a turn | `OpError::Invalid`, over HTTP `400` |
| a send or a create in a turn | `TurnError::Invalid` at the call |
| the new state of a turn | the message ends `failed` with the cause `handler`, as for a new state that is `null` |

An operator's retry of a message (`retry_message`, `POST /messages/{id}/retry`) checks the stored payload too: the
retry writes a new message, and without the check a retried `Failed` would grow one level with every failure.

- **A new `RetryResult::TooDeep`, answered `409 message-not-retryable` (chosen).** The request can be read; it is the
  stored message that cannot be retried, which is what `409` already says for a message that is pending or done. A
  bulk retry (`POST /messages/bulk-retry`) counts only what is accepted, so it skips such a message and goes on, as it
  does for one whose actor is gone. The cost is a variant of a public enum, in a release that breaks anyway.
- `OpError::Invalid`, answered `400`. No new variant, but `400` means a request that cannot be read (ADR 0026), and
  the bulk retry would have to catch that one error, as `?` would end the whole batch.

So
every value written from this release on is at most 64 deep, except a `Failed`, which is at most 65; every view is at
most 69 (a page of outcomes that holds a `Failed`).

The error names the payload or the state and the limit: `The payload is nested deeper than 64 levels`. The error of a
new state that a check refuses (`null`, a NUL character, too deep) starts with `State cannot be stored:` instead of
`State cannot be encoded:`, which stays for a state that does not encode.

Over HTTP the limit comes before the schema check, as the other limits do: a payload of 65 to 126 levels is `400`, not
`202` or `422`. It also comes before the repeated id, as the other limits do: a repeat of a send that a node of an
earlier release accepted with 65 to 126 levels gets `400`. `docs/architecture.md` (HTTP contract) says that the limits
come first, on every send, also one from code. The schema check keeps its own thread: a schema can still be deep.

A JSON value is dropped by serde_json with a walk that calls itself, so a `Value` built in code deep enough to
overflow that drop (thousands of levels) ends the process wherever it is dropped: in the host's code, or in cicada's
after a refusal. Not guarded: a guard would have to cover every path that holds the value, also a future dropped
before it runs, and the host's own code meets the same drop. The same holds for a Rust value that is too deep for
its own `Serialize`. The core guide says so in one line.

### 3. Values stored before keep working

A pending message or a state that was stored before this release and is deeper than 64 (up to 126 levels over HTTP,
any depth from code, and three or four levels more inside a ready-made actor) is delivered and decoded as before; a
view of it may be deeper than a parser that allows 128 reads, as it was. The limit is checked when a value is
written, not when it is read. A turn of such an actor that leaves its state as it is ends as before; a turn that
writes a state deeper than 64 fails (decision 2).

The upgrade notes say how to find such an actor (a failed message with `State cannot be stored: The state is nested
deeper than 64 levels`) and the two ways out: the host's handler writes a shallower state, or an operator deletes the
actor and creates it again, which also drops its mailbox and its children and tells its watchers `Stopped` (a
supervisor of it then forgets it as a child, as for any child that somebody else deletes). A state deeper than 126
levels, whose view `cicada-client` refuses, can be read with a parser that allows more, such as `curl` and `jq`.

A ready-made actor's handler is cicada's, so for it the notes name what works:

- **Dead letters** that hold a letter whose payload is deeper than 61 fail every turn that writes their state: no
  later failure is recorded, and `Redrive` and `Discard` of other letters fail. `Discard` of the deep letter itself
  works, as its state is then within the limit; the letter is the one whose payload is deeper than 61. Deleting and
  creating the actor works too, and drops all its letters.
- **A supervisor** whose state holds a child state deeper than 61 or a start payload deeper than 60 fails every turn
  that writes its state. Deleting it deletes its children too; it is created again with a shallower state, and
  starts them again.
- **A topic** with a pending `Publish` whose payload is deeper than 62 fails that `Publish`: its `Deliver` is refused
  at the call. The topic goes on with the next message.

No check before the upgrade finds such values; they show up as failed turns after it. Of the hosts known today,
Karakuri does not use cicada, and Chorus keeps the output of a task, which can be deep JSON from its users.

### 4. The ready-made actors and the limit

The ready-made actors are actors like any other: their states and the payloads of their messages are checked, and
they do not change.

- **Dead letters.** A letter holds a payload three levels into the state, so a failed message whose payload is
  deeper than 61 cannot be kept: the turn of the dead-letters actor that would record it fails with the cause
  `handler`, and no letter is recorded. Its outcome (`GET /outcomes`) holds the error and the id of the notification,
  for as long as outcomes are kept.
- **The topic.** A `Publish` holds its message two levels deep, so a payload deeper than 62 makes the `Publish`
  itself too deep: the publisher's send is refused at the call.
- **The supervisor.** A child entry holds the child's state three levels deep and the payload of its start message
  four: a supervisor whose state holds a child state deeper than 61, or a start payload deeper than 60, is refused
  when it is created.

The guide of the ready-made actors says so.

### 5. The decoder of an untagged enum that holds itself is named, not bounded

serde decodes such a type by trying each variant at each level, so for a value that does not decode the time
doubles with every level. A limit of depth does not help (2 to the 64th), and a decode that runs too long cannot be
stopped: it is code that does not yield.

The core guide says so, next to the limit: keep untagged enums that hold themselves out of messages and states, or
make them tagged. Over HTTP the schema check refuses most such values before they reach the decoder; not a value that
the schema accepts and only the decoder refuses (a number too large for its field), nor one below a nested message of
a family that holds itself, whose reference the door reads as "any value" (`schema-reference-names`).

### 6. The costs in proportion to a body are named

Both are linear in the size of a body, so one request cannot end a node with them; many large ones at once use
memory.

- The operations guide says that reading a body takes up to about 90 times its size, and the decoder of a node that
  hosts the type one copy more of the payload, and how to bound it: the body limit of the router (`ApiConfig`) and
  the number of requests at once.
- Not chosen: a limit on the number of values in a body; nobody has asked, and the body limit bounds it already.
- Not chosen: a decoder without the copy; the tag has to be put back into the payload for serde's internally tagged
  enum, and the copy is one more value of the size of the payload.

## Risks / Trade-offs

- A value deeper than 64 is refused from this release on. A host that sends such values must make them shallower.
  Chorus keeps the output of a task in the state of its workflow, three levels down: from this release on, a task
  output deeper than 61 levels fails the turn of the workflow that would keep it.
- The decoder of a type with large fields can need most of a thread's 2 MiB stack at 64 levels in a debug build: the
  type with six inline arrays of 32 numbers needs about 1.8 MiB. A release build needs far less. The core guide says
  so, without a figure, as no test measures it.
- A stored state deeper than 64 can no longer be written back by its actor (decision 3).
- A failed message with a payload deeper than 61 gets no dead letter (decision 4).
- A view of a value at the limit is deeper than the default of System.Text.Json and Json.NET (decision 1).
- A dead-letters actor that holds a letter deeper than the limit from before records nothing until that letter is
  discarded (decision 3).
- An operator cannot retry a message stored before the upgrade with a payload deeper than 64 (decision 2).

## Tests of values from before

Every write is checked from this release on, so a test that needs a value from before writes it past the checks:
the tests in `crates/cicada/src/tests` through the private backend, the tests of `cicada-actors` with raw SQL into a
SQLite store (the crate already uses sqlx for SQLite). The test of a repeat beyond the limit over HTTP uses a pending
`Failed` of a payload 64 levels deep, which the store writes 65 deep. The unit test of a value 100,000 levels deep
builds it without a walk that calls itself and leaks it (`std::mem::forget`), as its drop would overflow.

## Migration Plan

Version 0.16.0. No migration of the store; nodes of 0.15.0 and 0.16.0 can share a database. During a rolling deploy a
node of 0.15.0 still accepts a deeper value, and a client that repeats such a send to a node of 0.16.0 gets `400`, not
the answer to a repeated send, because the limits are checked before the repeated id.

1. A host that sends or stores values deeper than 64 levels makes them shallower before the upgrade.
2. After the upgrade, an actor whose turns fail with `State cannot be stored: The state is nested deeper than 64
   levels` gets a shallower state from its handler, or is deleted and created again; a dead-letters actor discards
   its deep letter (decision 3).

## Changed by the review

The first version took a refused value apart without calling itself, but a value held by an early return, or by a
future dropped before it runs, was dropped by serde_json all the same; decision 2 now says plainly that such a value
is the host's. It kept a dead letter without its payload and marked it with `null`, which a payload sent as raw JSON
can be as well; dead letters now stay as they are, and the failure stays in the outcomes. It also missed the spec
`http-schema-check` and its tests, which promise 126 levels; the outcomes page (four levels); the start message of a
supervisor (four levels); the order of the limits and the repeated id in a rolling deploy; and the clash with
`schema-reference-names` in decision 5. The error of a refused new state now says "stored", not "encoded".

The second review found that a dead-letters actor holding a deep letter from before would stop recording without an
error; decision 3 now names `Discard` of that letter, and what works for a supervisor. It found that an operator's
retry writes a stored payload without a check, which let a `Failed` grow a level with every retry; decision 2 now
checks it. It found a scenario that read a pending message in the outcomes, no test of the deepest view, a figure for
the guide that no test backs, the wrong wrapping of `GET /messages/{id}`, and a "no ADR" that rested on no earlier
case; ADR 0029 now records the rule. Karakuri was checked: it does not use cicada.

The third review found that a refused retry answered `400`, which means a request that cannot be read, and broke a
bulk retry; it is now a `RetryResult::TooDeep`, answered `409`. It found no plan for the tests of values from before
(now a section), the limit of about 122 missing from decision 1, the topic missing from the upgrade notes, the proposal
promising more than decision 2 gives, and a few lines that contradicted each other.

## Open Questions

None.
