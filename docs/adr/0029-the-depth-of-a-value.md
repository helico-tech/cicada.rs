# 0029. The depth of a value

Date: 2026-11-12

## Status

Accepted. It narrows ADR 0026: a value deeper than 64 levels is refused before the schema check, so the door checks
values of at most 64 levels against their schema, not 126.

## Context

A payload and a state are JSON, and JSON nests. Over HTTP the parser of a body allows 128 levels, so a value could have
126; code could build any depth. A value deep enough overflows the stack of a thread in a walk that calls itself (the
check of the limits, serde's encoder and decoder, the store), and the process ends without an error. The decoder of a
type with large fields overflows a debug build at 70 to 80 levels, and a view of a value at 126 levels is deeper than a
parser reads. The change `value-depth-limit` describes the measurements.

## Decision

- A payload and a state hold at most 64 arrays and objects inside each other. A scalar is 0 deep.
- The limit is checked where the other limits of a value are checked: for a send and a create from outside a turn
  (`OpError::Invalid`, over HTTP `400`, before the repeated id and before the schema check), for a send and a create
  in a turn (`TurnError::Invalid` at the call), for the new state of a turn (the message ends `failed`, cause
  `handler`), and for an operator's retry of a message (`409 message-not-retryable`; a bulk retry skips it). The
  check stops at the first level beyond the limit.
- A value stored before is read as before; only what is written is checked.
- The runtime writes a `Failed` that holds a payload of at most 64 levels one level down, so a notification can be 65
  deep. Every other value written from this decision on is at most 64 deep, and every view of one is read by a parser
  that allows 128 levels.
- The limit is one number, not a setting. 64 rather than the about 122 at which every view would still parse: the
  decoder of a type with large fields overflows a debug build at 70 to 80 levels, and wrappings need room.

## Consequences

- A host whose values nest deeper than 64 levels must make them shallower; a request that carried 65 to 126 levels
  gets `400` instead of `202` or `422`.
- A state stored deeper than 64 cannot be written back by its actor; the upgrade notes say how to get out.
- The ready-made actors hold values three or four levels into their own states and messages, so they keep values of
  60 to 62 levels at most.
- A `Value` built in code deep enough to overflow serde_json's own drop, or a Rust value too deep for its own
  `Serialize`, is beyond the check.
