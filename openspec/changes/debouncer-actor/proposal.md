# Proposal

## Why

A burst of events often needs one reaction: re-index after edits, send one notice after changes. ADR 0018 lists
"debounce" among the ready-made actors. Hosts can build it from the timer recipe, but need to get cancellation,
longest wait, replacement and failed delivery right. One actor per key provides those rules.

## What Changes

- Add `DebouncerActor`, type name `cicada.debouncer`, one instance per key, with fixed quiet and longest-wait settings.
- `Submit { to, message, retry }` keeps the latest target, StoredMessage, optional target retry policy and correlation
  id. It cancels the old timer and arms a public `Flush` after the quiet period, capped by the longest wait from the
  first handled submission. The timer carries the crate's shared retry policy.
- `Flush` delivers the currently kept value and clears it in the same commit; empty is a no-op. It also lets an
  operator deliver early or recover a failed flush through retry/redrive. A missing target leaves the value kept.
- A later Submit replaces the whole pending value and rearms it, also after a failed timer. It keeps the first
  submission's time until a flush succeeds. A superseded value is deliberately discarded, without notification.
- Never stop automatically. Hosts create instances explicitly, including create-and-submit in one turn.
- Pin the new contract, add a tested example and guide section, and host the type in the sample service without
  automatic instances. State the limits of deadlines during pauses/outages and the extra payload wrapping depth.

The simplest version is trailing delivery with a maximum scheduled wait. Public Flush deliberately bypasses the wait.

## Capabilities

### New Capabilities

- `debouncer`: retain the latest submission per key and forward it after quiet time, at the longest scheduled wait,
  or on explicit Flush, clearing it only when forwarding commits.

### Modified Capabilities

None.

## Out of scope

- Payload merging, a leading edge, live settings changes and automatic removal of idle instances.
- A guarantee about actual execution time, external rate limits, or downstream success.
- Changes to the core or a second delivery transaction.

## Order

After `rate-limiter-actor`, before `batcher-actor`.

## Impact

`crates/cicada-actors` (module, tests, exports and contract samples), `docs/reference/contracts`, the actors guide and
its doctests, a tested example, README, crate documentation, architecture's crate row, AGENTS map, changelog, and
sample-service catalog and contract snapshots/texts. No existing actor contract changes.
