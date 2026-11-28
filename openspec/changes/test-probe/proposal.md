# Proposal

## Why

`actor-ports` shows how to test an actor against a fake neighbour: a hand-written actor for the same ref. Each
consumer then writes the same kind of fake again and again: record what arrived, answer with a fixed reply. A ready
fake in `cicada-test` removes that work and makes tests of one actor quick to write. The owner asked for it on
2026-11-28 as a follow-up to `actor-ports`.

## What Changes

- `cicada-test` gains a probe: a fake actor for any `ActorRef`, hosted in `TestActors` in place of the real actor. It
  records each message it receives (decoded as the ref's message family, with its sender) and can answer each one
  with a reply chosen by a closure, sent to an address taken from the message or to its sender.
- Tests read what a probe received through `TestActors`, in arrival order.
- The `actor-ports` example's tests use the probe where it replaces a hand-written fake.

To decide in the design: where the recorded messages live (the probe's own state, read with `state`, or memory shared
with the test); how a reply closure names the reply address; whether a probe can answer with a refusal or a turn
failure; and the API names.

## Out of scope

- Probes outside `TestActors` (a node or app in production).
- Asserting on timing, retries or claims beyond what `TestActors` already shows.
- Changes to the runtime or the `cicada` core API.

## Order

After `work-id-header`. Before `worker-view`.

## Impact

Crates: cicada-test, cicada-examples.

A new public API in `cicada-test` (and in `cicada::testing` behind the `testing` feature, if that is where it lives),
its tests on virtual time, and the `actor-ports` example's tests. Docs handoff: the core guide's testing section and
the design guide's ports section.
