# Proposal

## Why

A payload or a state holds at most 64 levels of arrays and objects (ADR 0029). A message near that limit can still
fail, and what records its failure wraps the payload in more levels:

- The `Failed` notice holds the failed message's payload one level deeper. When a 62-level `Submit` to the debouncer
  fails, the watcher's turn fails as soon as it reads the notice (found while building `debouncer-actor`).
- Dead letters keep the failed message inside their own state, so they cannot keep a payload that deep either. A
  62-level batch item fails its publication, and neither the batcher nor the event can be recovered through dead
  letters (found while building `flow-events`; the actors guide says so today).

So a message that is valid when it is sent can fail in a way that nothing can watch or redrive.

## What Changes

- Make the records of a failure readable for every payload that was accepted when it was sent: the `Failed` notice
  and the state of dead letters. The design picks how: count the wrapper levels so that the limit applies to the
  payload, not to its wrapper, or hold the failed payload in a form that does not add levels.
- Tests at the boundary: a 64-level payload that fails is watched and kept by dead letters, and its redrive delivers
  it unchanged.
- Two small repairs in the same area, found in earlier reviews:
  - The doc comment on `JournalEntry::Sleep` (`crates/cicada-actors/src/flow.rs`) still says the sleep is reserved
    for a later release. It shipped in 0.16.6. The comment is part of the pinned contract text, so this regenerates
    the contract fixtures.
  - The event's lifecycle test (`crates/cicada-actors/tests/event.rs`, about line 648) deletes the event before its
    queued `Unsubscribe` runs, so it never tests unsubscribing while runs exist. Run the `Unsubscribe`, assert its
    outcome and the kept runs, then delete.

## To decide

- Which of the two ways above, and whether it changes a wire format or a contract hash (then it needs an ADR and is
  breaking).
- Whether the guide's warnings in the batcher and event sections go away or change.

## Out of scope

Raising the 64-level limit itself; deeper limits for schemas (`MAX_SCHEMA_DEPTH`, 0.16.12); other changes to dead
letters or to watches.

## Order

After `app-worker-over-http`. Before `listener-probe-on-close`.

## Impact

`crates/cicada` (watch notices, the depth check), `crates/cicada-actors` (dead letters, flow, event tests), contract
fixtures and the actors guide.
