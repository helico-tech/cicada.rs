# Proposal

## Why

A burst of events often needs one reaction: re-index after a run of edits, send one notice after a run of changes.
ADR 0018 lists "debounce" among the ready-made actors that come later. A host can build it from the core's timer
recipe (keep the id of the armed timer, cancel it and arm a new one), but the details are easy to get wrong.

Outside, Inngest, DBOS, Trigger.dev, graphile-worker and pg-boss ship a debounce, and all mean the same: per key, the
last message wins, each new message restarts a quiet period, and a longest wait stops steady input from putting it
off forever. graphile-worker shipped without the longest wait, and its users have an open request for it.

## What Changes

- A new ready-made actor `DebouncerActor`, type name `cicada.debouncer`, one instance per key.
- Its message `Submit { to, message }` takes a target address and a `StoredMessage`. It keeps the message (the last
  one wins), cancels the armed `Fire` and arms a new one after the quiet period, but no later than the longest wait,
  counted from the first message of the burst.
- Its own `Fire` sends the kept message to the target and clears it in the same commit. A target that is absent fails
  the `Fire`, and the message stays kept.
- Its settings are in its state, set at create: the quiet period and the longest wait, both required.
- It never stops itself. A turn that stops its actor purges the other pending messages, and a purge tells nobody, so
  a `Submit` that arrives while it fires would be lost.
- A caller can create an instance and send to it in one turn, since a commit applies creates before sends.
- Its contract is pinned in `docs/reference/contracts`; the actors guide gets a section with doctests.

The simplest version: trailing delivery only, the last message wins. A throttle that delivers at most one message per
period is a debouncer whose longest wait equals its quiet period, so it needs no actor of its own.

## Capabilities

### New Capabilities

- `debouncer`: a ready-made actor that passes on only the last message of a burst, after a quiet period or at a
  longest wait, and never loses a message by stopping itself.

### Modified Capabilities

None.

## Out of scope

- Merging the payloads of a burst; the last one wins.
- Delivering the first message at once and holding back the rest (a leading edge).
- Removing idle instances by itself; a host deletes them.
- Changes to the core.

## Questions for the design

- Does `Fire` send to the target itself, or through a turn of its own with the crate's shared retry policy, as the
  topic (`Deliver`) and the scheduler (`Run`) do? A delivery the debouncer counts only from itself cannot be retried
  or redriven by an operator; the scheduler's `Run` is public for that reason.
- Does a `Submit` to a different target in the middle of a burst replace the target too, or is that refused?

## Order

After `rate-limiter-actor`, before `batcher-actor`.

## Impact

- `crates/cicada-actors` (a module `debouncer`, its tests on virtual time, `tests/wire_formats.rs`),
  `docs/reference/contracts` (a new file), [docs/guide/actors.md](../../../docs/guide/actors.md) and its doctests,
  `README.md` (the table of actors), `CHANGELOG.md`.
- Hosts: no host has asked for it.
