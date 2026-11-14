# Proposal

## Why

A payload or a state holds at most 64 levels of arrays and objects (ADR 0029). A message near that limit can still
fail, and what records its failure wraps the payload in more levels:

- The `Failed` notice holds the failed message's payload one level deeper. Investigation confirms that a typed
  watcher can read it; the debouncer test's witness fails when it copies the notice into its own state. Pin this
  boundary in tests, including remote workers, and correct the misleading comment.
- Dead letters keep the failed message inside their own state, so they cannot keep a payload that deep either. A
  62-level batch item fails its publication, and neither the batcher nor the event can be recovered through dead
  letters (found while building `flow-events`; the actors guide says so today).

So a message that is valid when sent can be watched but cannot currently be kept and redriven by dead letters.

## What Changes

- Keep the core notice unchanged. Dead letters store payloads deeper than 61 levels as JSON text in `payloadJson`,
  mutually exclusive with the existing raw `payload` field. No marker inside user data is reinterpreted.
- Enable exact floating-point text parsing for serde_json so the added parse does not change the captured value.
- Change the dead-letter Rust API and contract, with an explicit backup/restore upgrade that preserves saved letters;
  this requires a breaking release and ADR 0043, but no store schema migration.
- Tests at the boundary: a 64-level payload that fails is watched and kept by dead letters, and its redrive delivers
  it unchanged.
- Two small repairs in the same area, found in earlier reviews:
  - The doc comment on `JournalEntry::Sleep` (`crates/cicada-actors/src/flow.rs`) still says the sleep is reserved
    for a later release. It shipped in 0.16.6. The comment is part of the pinned contract text, so this regenerates
    the contract fixtures.
  - The event's lifecycle test (`crates/cicada-actors/tests/event.rs`, about line 648) deletes the event before its
    queued `Unsubscribe` runs, so it never tests unsubscribing while runs exist. Run the `Unsubscribe`, assert its
    outcome and the kept runs, then delete.

## Decisions

The design records the field discriminator, exact parsing, ordinary depth checks on redrive, and the upgrade boundary.
The batcher and event limits stay; dead letters can keep a refused publication, but retrying it does not cure the cause.

## Out of scope

Raising the 64-level limit itself; deeper limits for schemas (`MAX_SCHEMA_DEPTH`, 0.16.12); other changes to dead
letters or to watches.

## Order

After `app-worker-over-http`. Before `listener-probe-on-close`.

## Impact

`crates/cicada` (watch notices, the depth check), `crates/cicada-actors` (dead letters, flow, event tests), contract
fixtures and the actors guide.
