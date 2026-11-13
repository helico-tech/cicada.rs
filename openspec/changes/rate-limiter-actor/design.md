# Design

## Scope and decisions

One new actor on the public core API; no core or store changes. The owner agreed A1–F1 on 2026-11-13;
the remaining choices were settled with the owner's Opus 5.5 adviser. The owner asked to design the whole queue,
overriding the usual two-ready-change cap. This change fits four implementation tasks plus finishing checks.

### A1–B1. Fixed settings; scheduled delivery only

`RateLimiterActor` is a unit struct. `RateLimiter { name }`, with `named`, names `cicada.rate-limiter/<name>`.
The rate, burst and longest delay are fixed at creation. There is no `SetRate`: no host needs live changes,
and it could not reschedule forwarded messages. Replacing the limiter resets its reservations, not target mailboxes.
A later setter or state field needs a contract migration or a versioned type. There are no counters or watches.

This actor assigns due times, not execution times or external API admission. Overdue messages can run close together
after a pause, outage or slow target. Target retries bypass the limiter. The guide states these limits.
Admission at the external call was rejected: it needs a different worker protocol.

### C1. Explicit creation

Hosts use `ActorSystem::create`; a producer can `turn.create(&limiter, &state)` then `turn.send(&limiter, submit)`
in one commit. Existing instances keep their state, so producers sharing a key must agree on settings.
The guide shows both forms. No create-and-submit helper: two core calls suffice.

### D1 and F1. State, precision and bounds

`RateLimiterState` has public fields `count: u32`, `period: Duration`, `burst: u32`, `max_delay: Duration`,
and `idle_at: Option<Timestamp>` (missing/null means fully idle). Serde uses camelCase and ISO-8601 durations.
`RateLimiterState::new(count, period, burst, max_delay) -> Result<Self, InvalidRate>` sets `idle_at` to `None`.
A checked serde conversion validates construction and decoding alike; unknown fields follow ordinary derived serde
and are ignored. `InvalidRate` implements `std::error::Error`. Example initial JSON:
`{"count":2,"period":"PT1S","burst":2,"maxDelay":"PT1S"}`.

Count and burst are positive; `0 < period <= MAX_DELAY`, `0 <= maxDelay <= MAX_DELAY`.
Compute the interval `I = ceil(period.as_nanos() / (count * 1000))` in integer arithmetic, in microseconds.
A positive sub-microsecond period still gives `I = 1`; 3 per second gives 333334 microseconds.
The burst allowance `tau = (burst - 1) * I` must not exceed `MAX_DELAY`, using checked multiplication.
A supplied `idleAt` must be at most `MAX_INSTANT`; round it up to microseconds when calculating a slot.
Use checked arithmetic and checked timestamp conversion; no panic, wrapping or saturation to an earlier slot.
Not chosen: exact divisibility (rejects ordinary rates), fractional reservations (more bookkeeping), or a fixed burst cap.

### D1. One reservation and one send in one commit

In microseconds, `now = turn.now()`, `base = max(now, ceil(idleAt).unwrap_or(now))`,
`slot = max(now, base - tau)`, and the new `idleAt = base + I`. Check all arithmetic and both resulting timestamps
against `MAX_INSTANT` before buffering the send. The timestamp is when the full burst is free again, not the next slot.
A slot whose delay exceeds `maxDelay` is refused permanently; equality is accepted. Count delay from handling,
not enqueueing or commit. Compare against the full duration, without rounding `maxDelay` upward.
At 2/s, burst 2, maxDelay 1 s, five simultaneous submissions get now, now, +0.5 s, +1 s, refused.
Burst 1 means strict slot spacing; zero maxDelay admits only immediate capacity. Idle time replenishes the burst.
The state and send commit atomically. Refusal consumes no slot; cancelling a forwarded message refunds none.
`SendOptions::at` clamps a past slot to store time at commit. A slow registry read or commit can therefore narrow
the spacing between stored deliverAt values too; the guide names this alongside overdue target messages.
Not chosen: burst as extra messages beyond the first (misleading count), or a local queue and background drip.

### E1. Submit and the forwarded envelope

The family is `RateLimiterMsg::Submit { to: ActorAddress, message: StoredMessage, retry: Option<RetryPolicy> }`.
It uses the crate's tagged/camelCase form; missing/null retry means none. The typed helper
`RateLimiterMsg::submit(&target_ref, message, retry) -> Result<Self, EncodeError>` checks the ref and its family.
The forwarded message gets a new id, the limiter as sender, the submission's correlation id, default retention,
and observer headers only. `retry` governs the target; the submission envelope's policy governs the limiter.
Not chosen: copying that envelope policy (couples two kinds of work), or omitting target retries (later contract change).
A reply address belongs in the payload; the sender is not the original caller. Payloads fit the ordinary depth limit:
`message.payload` leaves at most 62 levels for the inner value, without an actor-specific depth check.

### Failures and recovery

Delay refusal, arithmetic overflow and a send rejected at the call are permanent (`refused`). An absent instance
rejects the commit (`commit`), not retried. Neither advances state. A store read error is propagated normally,
so the submission's policy decides retries; without one it fails. Any terminal failure tells watchers by core rules.
A target's decoding or handling failure happens after forwarding and refunds no slot. No target watch is installed.
Public `Submit` works on operator retry or dead-letter redrive, calculating a fresh slot from that turn's store time.
An incoming duplicate id is handled by the core's dedup rules. There is no extra dedup field or sender check.

### Integration and verification

Pin the new contract and samples in `wire_formats.rs` and `docs/reference/contracts`. Register the type in the sample
service, with no startup instance, and update its contract snapshots/texts. Add a tested example with guide doctests;
update the guide tables, README, crate docs, architecture's crate row, AGENTS map and changelog. No new ADR: the
scheduler design settled that an added actor contract following ADR 0028 alters no existing wire format or guarantee.

Virtual-time `TestActors` tests cover spacing, bursts, bounds, rollback, retry/redrive, envelope and creation;
unit tests cover validation and checked arithmetic. The spec names boundary and failure scenarios. The existing SQLite
registry-failure fixture can cover store-error propagation if the public virtual-time harness cannot inject it.
The builder runs targeted checks while implementing and the configured finishing checks once at the end.

Review 1 found no blockers. Its note about clamping at commit is incorporated above and in a spec scenario.
