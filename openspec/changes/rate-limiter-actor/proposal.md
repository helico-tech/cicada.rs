# Proposal

## Why

A host may want many senders to share a rate for scheduled delivery. In cicada one turn at a time per instance is
free (G1), but shared delivery slots are not. ADR 0018 lists "rate limit" among the ready-made actors that come later.
This actor spaces when messages become due; it does not enforce an outside API's quota at the time of the call.

Outside, rate limiting is a paid feature of Sidekiq Enterprise, Oban Pro and Hangfire Ace, and it was the most
requested issue of GoodJob. Platforms built like cicada, with one writer per key, ship it as a keyed object with
delayed sends (Restate documents it as a recipe).

## What Changes

- A new ready-made actor `RateLimiterActor`, type name `cicada.rate-limiter`, one instance per limit (the key is its
  name).
- Its one message `Submit { to, message, retry }` takes a target address, a `StoredMessage` and an optional target
  retry policy, defaulting to none. The actor computes the next free slot and sends the message to the target at
  that slot (`SendOptions::at`). The messages wait in the target's
  mailbox; the limiter's state holds no queue.
- The slot comes from GCRA (the generic cell rate algorithm): the state keeps one timestamp, with no background drip.
  Time is the store clock (`turn.now()`); the interval is rounded up to a whole microsecond.
- Its settings are fixed in its state at create: the rate (a count per period), the burst, and the longest delay. A
  `Submit` whose slot is beyond the longest delay fails for good, so it reaches the dead letters where they watch.
- Burst is the total immediate capacity, at least 1. A slot exactly at the longest delay is accepted; a zero delay
  permits only immediately available capacity. The delay is counted from handling the submission.
- Hosts create shared limiters at startup, or an actor creates one and submits in the same turn. Repeated creates
  leave an existing limiter's state and settings untouched.
- A `Submit` to a target that is absent fails, and the timestamp does not move.
- A forwarded message carries the limiter as its sender and preserves the submission's correlation id. It has a
  new id, default retention, and headers from observers rather than copied incoming headers. A reply address belongs
  in the payload; the guide says so. Its retry policy is independent of the submission's envelope policy.
- Its contract is pinned in `docs/reference/contracts`; the actors guide gets a section with doctests.
- The sample service hosts the type without creating a limiter automatically.

The simplest version: one rate per instance, shaping only. Nothing asks the limiter "may I?".
Overdue messages can run close together after a target resumes or a node recovers. The guide states this limitation.

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
- Enforcing spacing between actual handler runs or external calls, including after a pause or outage.
- Changing settings with a message. There is no `SetRate`; replacing a limiter does not reschedule messages already
  forwarded to targets.
- Several limits inside one instance; use one instance per limit.
- A message that the target retries goes back after its own backoff, without passing the limiter again.
- Jitter, and changes to the core.

## Order

After `scheduler-actor`, before `debouncer-actor`.

## Impact

- `crates/cicada-actors` (a module `rate_limiter`, its tests on virtual time, `tests/wire_formats.rs`),
  `docs/reference/contracts` (a new file), [docs/guide/actors.md](../../../docs/guide/actors.md) and its doctests,
  a tested example in `examples/`, `README.md` (the table of actors), `CHANGELOG.md`, `docs/architecture.md`
  (the crate row), `AGENTS.md` (the map), and the sample service's catalog and contract snapshots/texts.
- Hosts: no host has asked for it.
