# Proposal

## Why

A host that calls an outside API with a quota must space its calls out. In cicada one call at a time per key is free,
because the turns of one actor never overlap (G1), but a rate shared by many senders is not. ADR 0018 lists "rate
limit" among the ready-made actors that come later.

Outside, rate limiting is a paid feature of Sidekiq Enterprise, Oban Pro and Hangfire Ace, and it was the most
requested issue of GoodJob. Platforms built like cicada, with one writer per key, ship it as a keyed object with
delayed sends (Restate documents it as a recipe).

## What Changes

- A new ready-made actor `RateLimiterActor`, type name `cicada.rate-limiter`, one instance per limit (the key is its
  name).
- Its one message `Submit { to, message }` takes a target address and a `StoredMessage`. The actor computes the next
  free slot and sends the message to the target at that slot (`SendOptions::at`). The messages wait in the target's
  mailbox; the limiter's state holds no queue.
- The slot comes from GCRA (the generic cell rate algorithm): the state keeps one timestamp, with no background drip.
  Time is the store clock (`turn.now()`).
- Its settings are in its state, set at create: the rate (a count per period), the burst, and the longest delay. A
  `Submit` whose slot is beyond the longest delay fails for good, so it reaches the dead letters where they watch.
- A `Submit` to a target that is absent fails, and the timestamp does not move.
- A forwarded message carries the limiter as its sender. A reply address belongs in the payload; the guide says so.
- Its contract is pinned in `docs/reference/contracts`; the actors guide gets a section with doctests.

The simplest version: one rate per instance, shaping only. Nothing asks the limiter "may I?".

## Capabilities

### New Capabilities

- `rate-limiter`: a ready-made actor that passes each message to its target no earlier than the next free slot of a
  rate, keeps one timestamp, and refuses a message that would wait longer than its longest delay.

### Modified Capabilities

None.

## Out of scope

- A limit on how many run at once (a semaphore). It needs a release protocol in the worker's code, or a notice of
  success that the core does not give.
- A check that answers whether a call may go now. Cicada has no request and response.
- Several limits inside one instance; use one instance per limit.
- A message that the target retries goes back after its own backoff, without passing the limiter again.
- Jitter, and changes to the core.

## Questions for the design

- Is a `SetRate` message in the first contract? A node refuses to start when a contract changes, so adding it later
  needs a new type name.
- How does a host make sure the limiter exists before the first send? A send to an absent actor rejects the sender's
  commit; the sample service creates its singletons at start.

## Order

After `scheduler-actor`, before `debouncer-actor`.

## Impact

- `crates/cicada-actors` (a module `rate_limiter`, its tests on virtual time, `tests/wire_formats.rs`),
  `docs/reference/contracts` (a new file), [docs/guide/actors.md](../../../docs/guide/actors.md) and its doctests,
  `README.md` (the table of actors), `CHANGELOG.md`.
- Hosts: no host has asked for it.
