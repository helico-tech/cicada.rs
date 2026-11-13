# rate-limiter Specification

## Purpose
A rate limiter shared by its senders forwards each submitted message to its target at the next free slot of a rate
with a burst, and refuses a message whose slot lies further away than its longest delay.

## Requirements

### Requirement: One limiter keeps one rate and one reservation timestamp

The actor SHALL use type name `cicada.rate-limiter`, with one instance per name. Its state SHALL contain `count`,
`period`, `burst`, `maxDelay` and optional `idleAt`. The settings SHALL be fixed at creation. `idleAt` SHALL be the
time at which the full burst becomes available, absent or null for a fresh limiter. No queue or counters are kept.
Durations SHALL use the core's ISO-8601 form.

#### Scenario: Minimal initial state

- **WHEN** a host creates a limiter with `{"count":2,"period":"PT1S","burst":2,"maxDelay":"PT1S"}`
- **THEN** it has its full burst available without a start message

### Requirement: Invalid settings are refused

The typed constructor and state decoder SHALL enforce positive u32 count and burst, a positive period at most
`MAX_DELAY`, a maxDelay from zero through `MAX_DELAY`, and a burst allowance at most `MAX_DELAY`.
A supplied idleAt SHALL NOT exceed `MAX_INSTANT`.

#### Scenario: Invalid settings

- **WHEN** construction or decoding receives zero count, zero burst, zero period, a duration over its bound,
  a burst allowance over MAX_DELAY, or idleAt beyond MAX_INSTANT
- **THEN** it returns an error rather than accepting those settings

### Requirement: Creation uses the existing core API

A host SHALL be able to create a limiter through `ActorSystem::create`, or create and submit in one producer turn.
A repeated create SHALL leave an existing limiter's settings and reservations unchanged.

#### Scenario: Atomic create and submit

- **WHEN** a producer creates a new limiter and sends it a Submit in the same turn
- **THEN** both effects commit together and that submission can reserve a slot

#### Scenario: Repeated create

- **WHEN** a producer creates an existing limiter with different settings and submits
- **THEN** the original settings and reservation timestamp determine its slot

### Requirement: Slot arithmetic is conservative and checked

The interval I SHALL be `ceil(period.as_nanos() / (count * 1000))` whole microseconds. Let tau be `(burst - 1) * I`,
now be the store time at claim, and base be `max(now, ceil(idleAt))`, using now when idleAt is absent.
The slot SHALL be `max(now, base - tau)` and the new idleAt SHALL be `base + I`.

#### Scenario: A rate that needs rounding

- **WHEN** a fresh limiter at 3 per second with burst 1 handles three submissions at the same store time
- **THEN** the slots are now, now plus 333334 microseconds, and now plus 666668 microseconds

#### Scenario: Sub-microsecond period

- **WHEN** a limiter has count 1, period 1 nanosecond, and burst 1
- **THEN** consecutive reservations at the same store time are one microsecond apart

#### Scenario: Fractional supplied timestamp

- **WHEN** a limiter with burst 1 has idleAt half a microsecond after now and receives a submission
- **THEN** its slot rounds up to one microsecond after now

### Requirement: Timestamp arithmetic cannot overflow

The slot and new idleAt SHALL fit through MAX_INSTANT. Checked arithmetic or conversion overflow SHALL refuse the
submission permanently with no effects, rather than panic, wrap or move a slot earlier.

#### Scenario: Timestamp overflow

- **WHEN** a valid limiter receives a submission whose slot or new idleAt would exceed MAX_INSTANT
- **THEN** the submission fails permanently, the state stays unchanged, and nothing is forwarded

### Requirement: Burst and longest delay bound admission

Burst SHALL count the total immediately available messages after sufficient idle time, not extras after the first.
A submission SHALL be refused permanently if its slot is more than maxDelay after the store time at claim.
Equality SHALL be accepted. The comparison SHALL NOT round maxDelay upward or include time waiting in the mailbox.

#### Scenario: Burst and inclusive delay

- **WHEN** a fresh limiter at 2 per second, burst 2 and maxDelay 1 second handles five submissions at the same time
- **THEN** it forwards four with slots now, now, plus 0.5 seconds and plus 1 second, and refuses the fifth
- **AND** the fifth consumes no capacity and idleAt stays at now plus 2 seconds

#### Scenario: Immediate capacity only

- **WHEN** a fresh limiter at 1 per second, burst 1 and maxDelay zero handles two simultaneous submissions
- **THEN** the first is forwarded immediately and the second fails permanently even if its envelope requests retries

#### Scenario: Idle replenishment

- **WHEN** a limiter at 2 per second with burst 2 uses its initial burst and then handles two submissions a second later
- **THEN** both new submissions are immediately eligible

#### Scenario: Sub-microsecond longest delay

- **WHEN** a limiter's next slot is one microsecond away and maxDelay is half a microsecond
- **THEN** it refuses the submission

### Requirement: A submission forwards once with its selected target policy

The outgoing message SHALL contain the stored type and payload, a new id, the limiter as sender, the submission's
correlation id and the selected target retry policy. Retention SHALL use the default. Incoming headers SHALL NOT be
copied; ordinary observer headers still apply. The timestamp and outgoing message SHALL commit atomically.

#### Scenario: Target envelope

- **WHEN** a submission has correlation id c, arbitrary header h, its own retry policy P, and target retry policy Q
- **THEN** the forwarded message has correlation id c, the limiter as sender, a distinct id and retry policy Q
- **AND** it has default retention and does not inherit h, while observer headers can be present

### Requirement: Submit selects the target policy independently

The only message SHALL be public `Submit { to, message, retry }`, where message is a StoredMessage and retry is an
optional target RetryPolicy, absent or null for none. The submission envelope's policy SHALL govern only its own turn.
A typed helper SHALL check the target ref's message family at compile time.

#### Scenario: No target retry requested

- **WHEN** a submission with an envelope retry policy omits its target retry and is forwarded
- **THEN** the target message has no retry policy

#### Scenario: Target retries bypass the limiter

- **WHEN** a forwarded message with target retries fails once at its target
- **THEN** the target retries by that policy without another reservation or a change to the limiter's state

#### Scenario: Wrong typed family

- **WHEN** code uses the submit helper with a message from another ref's family
- **THEN** it fails to compile

### Requirement: A failed forwarding consumes no slot

A send rejected at the call SHALL fail permanently (cause refused). An absent target instance SHALL reject the
commit (cause commit). A store read error SHALL propagate as a retryable handler error, governed by the submission's
envelope policy. No failed forwarding SHALL advance idleAt. Terminal failures SHALL notify watchers by core rules.
A failure at the target after the forwarding committed SHALL NOT refund capacity.

#### Scenario: Absent target

- **WHEN** a submission targets an absent instance of a registered type
- **THEN** it fails with cause commit, sends nothing and leaves idleAt unchanged
- **AND** a following submission to a present target gets the unconsumed slot

#### Scenario: Refused target message type

- **WHEN** a submission names an unregistered target type or a message type that its target does not accept
- **THEN** it fails permanently with cause refused and leaves idleAt unchanged

#### Scenario: Temporary store failure

- **WHEN** a submission with a retry policy cannot read the registry, and the store recovers before its next attempt
- **THEN** the failed attempt changes no reservation and the retried attempt forwards once at a newly calculated slot

#### Scenario: Target rejects the payload

- **WHEN** forwarding commits but the target cannot decode the payload
- **THEN** the target message fails and the limiter's reservation remains consumed

### Requirement: Core recovery and control retain their meaning

A retried or redriven Submit SHALL be handled without a sender or id restriction and SHALL calculate its slot from
that turn's store time. Repeated accepted ids SHALL use the core's dedup rules. Deleting the limiter SHALL NOT remove
messages it already forwarded to independent targets. Cancelling one of those messages SHALL NOT refund its slot.

#### Scenario: Operator recovery

- **WHEN** a submission failed because the target was absent, the target is created, and the submission is retried
- **THEN** the limiter computes a fresh slot and forwards it

#### Scenario: Dead-letter redrive

- **WHEN** a watched limiter refused a submission for excessive delay and dead letters redrive it after idle time
- **THEN** the limiter handles it normally and can admit it using the recovered capacity

#### Scenario: A repeated incoming id

- **WHEN** a client repeats an accepted submission's id within the core's dedup window
- **THEN** no second forwarding or reservation is made

#### Scenario: Cancelling a target message

- **WHEN** an operator cancels a forwarded message before it runs
- **THEN** the limiter's idleAt is unchanged

### Requirement: The guide states the scheduling limits

The guide SHALL explain that slots constrain due times, not actual executions, and maxDelay excludes mailbox and
commit delays. It SHALL show startup creation, atomic create-and-submit, target retry selection and a reply address
in the payload. The sample service SHALL host the type without creating a limiter automatically. The contract SHALL
be pinned alongside the other ready-made actors.

#### Scenario: Overdue target messages

- **WHEN** a paused target accumulates messages with spaced due times and resumes after all are due
- **THEN** it may handle them close together, without the guide promising external API quota enforcement

#### Scenario: A replacement limiter

- **WHEN** a host deletes a limiter and creates it with new settings
- **THEN** its reservations start fresh and previously forwarded messages retain their due times

#### Scenario: A slow commit narrows delivery spacing

- **WHEN** a limiter with a one-second interval reserves a slot at time zero but commits at time two seconds,
  then handles and commits another submission at time two seconds
- **THEN** the core clamps the first message's due time to two seconds and the second may also be due then
- **AND** the guide states that assigned slots do not guarantee spacing between stored deliverAt values
