# debouncer Specification

## Purpose
A debouncer keeps only the latest message of a burst and sends it to its target after a quiet period, at the latest
a longest wait after the burst's first submission, and a public Flush sends what it keeps at once.

## Requirements

### Requirement: A debouncer has fixed valid settings

The actor SHALL use type name `cicada.debouncer`. Its state SHALL contain quiet, maxWait, optional pending, firstAt
and armed. Settings SHALL satisfy `0 < quiet <= maxWait <= MAX_DELAY` at construction and decoding. Pending SHALL
require firstAt; empty SHALL have no firstAt or armed. Pending without armed SHALL be valid. FirstAt SHALL NOT exceed
MAX_INSTANT. An initial state with only settings SHALL be empty. Durations SHALL use ISO-8601.

#### Scenario: Invalid settings or inconsistent state

- **WHEN** a state has zero quiet, maxWait below quiet, a duration over MAX_DELAY, pending without firstAt,
  empty pending with firstAt or armed, or firstAt beyond MAX_INSTANT
- **THEN** construction or decoding refuses it

#### Scenario: Explicit creation

- **WHEN** a producer creates a debouncer with quiet 2 seconds and maxWait 5 seconds and submits in one turn
- **THEN** the core commits both effects and the debouncer can handle that submission
- **AND** repeating the create leaves its settings and pending state unchanged

### Requirement: The entire latest submission replaces the previous one

Submit SHALL contain to, a StoredMessage and optional target retry, absent/null for none. Handling SHALL keep these
with the submission's correlation id and replace the previous pending value. A typed submit helper SHALL check the
ref's message family. Superseded submissions SHALL end done without a failure notification. No target lookup SHALL
occur until Flush.

#### Scenario: A new target and policy

- **WHEN** a burst starts with a message to target A, then receives a message to B with another retry and correlation id
- **THEN** the pending value contains B and all of the second submission's values
- **AND** A receives no delivery from that burst

#### Scenario: A target not yet present

- **WHEN** a Submit names an absent instance or unregistered type
- **THEN** it can be kept; target validation waits until Flush

### Requirement: Submit replaces the timer but keeps the burst's start

The first handled Submit SHALL set firstAt to store time at claim. Later submissions SHALL keep it. Each SHALL cancel
the previous armed message, except its own id, and arm Flush at `min(now + quiet, firstAt + maxWait)` with the shared
retry policy. Replacing state and timer SHALL commit together. Unsupported chosen times or arithmetic overflow SHALL
fail permanently without changing the old state or timer.

#### Scenario: Quiet delivery

- **WHEN** quiet is 2 seconds and one submission is handled at time zero
- **THEN** its Flush is due at 2 seconds and no automatic delivery occurs earlier

#### Scenario: Continuous submissions reach the cap

- **WHEN** quiet is 2 seconds, maxWait is 5 seconds and submissions are handled at 0, 1, 2, 3 and 4 seconds
- **THEN** only the last value is delivered by the timer due at 5 seconds when turns run promptly

#### Scenario: A new burst

- **WHEN** a burst is successfully flushed and a new Submit is handled at time 10 seconds
- **THEN** its firstAt is 10 seconds and its deadline is computed from that new burst

#### Scenario: Submit and timer have the same due time

- **WHEN** quiet is 2 seconds, maxWait is 5 seconds, and a new Submit and the first burst's Flush are due at 2 seconds
- **THEN** the core's sequence order decides: Submit first replaces the kept value and cancels that timer;
  Flush first delivers the old value and the Submit starts a new burst
- **AND** in either order the new value's timer is due at 4 seconds

#### Scenario: Timestamp overflow

- **WHEN** handling a Submit cannot represent its chosen deadline through MAX_INSTANT
- **THEN** it fails permanently and the earlier pending value and timer remain unchanged

### Requirement: Public Flush delivers and clears atomically

Any Flush SHALL send the current pending message to its target, cancel armed unless it is the current message id,
and clear pending, firstAt and armed in the same commit. Empty Flush SHALL end done without effects. No sender or id
gate SHALL restrict it. The actor SHALL NOT stop itself after flushing.

#### Scenario: A manual early Flush

- **WHEN** an operator sends Flush before the timer is due
- **THEN** it forwards the pending value immediately, clears the burst and cancels the timer

#### Scenario: The timer flushes itself

- **WHEN** the armed Flush runs with pending data
- **THEN** it forwards and clears successfully without trying to cancel its own message

#### Scenario: Empty Flush

- **WHEN** an empty debouncer receives Flush
- **THEN** it ends done, sends nothing and remains available for later submissions

### Requirement: Forwarding retains the latest submission's selected context

The target message SHALL have the stored type and payload, a new id, the debouncer as sender, the kept correlation id,
the kept target retry and default retention. Incoming Submit headers SHALL NOT be kept or copied; normal Flush-turn
observer headers SHALL still apply. Target retries SHALL bypass the debouncer.

#### Scenario: Distinct target and timer policies

- **WHEN** Submit keeps target policy P and Flush has envelope policy Q
- **THEN** P governs the target and Q governs the flush, with no mixing of those policies
- **AND** the target sees the kept correlation id, not the manual Flush's correlation id

### Requirement: A failed Flush leaves pending data available for recovery

A call-level rejection SHALL fail Flush permanently; an absent target instance SHALL reject its commit. Store read
errors SHALL follow its envelope retry policy. Failure SHALL NOT clear or replace pending data or cancel another
timer. Terminal failures SHALL notify watchers under core rules. The debouncer SHALL NOT watch its target.

#### Scenario: Absent target and recovery

- **WHEN** the timer's Flush fails at commit because its target is absent, then the target is created and Flush is retried
- **THEN** the kept value is forwarded once and the state clears

#### Scenario: Rejected message type

- **WHEN** the kept target type is unregistered or does not accept the stored message type at Flush
- **THEN** Flush fails permanently with cause refused and pending data stays

#### Scenario: Temporary store error

- **WHEN** the timer cannot read the registry and the store recovers before its retry
- **THEN** the retry forwards the kept value once, with no earlier state clearing

#### Scenario: Replacement cancels a retrying timer

- **WHEN** a timer Flush has been rescheduled after a store error and a new Submit is handled before that retry
- **THEN** the old retry is cancelled and only the replacement timer remains armed

#### Scenario: Target failure after forwarding

- **WHEN** forwarding commits but the target cannot decode the payload or its handler fails
- **THEN** the debouncer remains cleared; only the target's message fails or retries

### Requirement: Timer loss and redrive act on current state

A lost timer SHALL NOT automatically clear pending data. A new Submit SHALL replace it and rearm from the original
firstAt, with a past deadline clamped by the core. Manual Flush, operator retry and dead-letter redrive SHALL act on
the current pending value, including a later burst. The guide SHALL state this and the effects of cancel and purge.

#### Scenario: Rearming after the cap

- **WHEN** a timer failed or was cancelled or purged, maxWait has passed, and a new Submit is handled
- **THEN** it replaces the pending value and arms Flush due immediately

#### Scenario: Redrive into a later burst

- **WHEN** an old failed Flush is redriven while a new burst is waiting
- **THEN** it flushes the new burst immediately, rather than restoring the old value

### Requirement: Deep pending values fail without losing the earlier burst

The existing core state-depth check SHALL remain authoritative. Pending adds three container levels before the
inner payload, leaving at most 61. A Submit that fits its own envelope but would exceed the state limit SHALL fail
at the post-handler state check, with no committed replacement or timer effects. No extra recursive validator is added.

#### Scenario: Payload depth boundary

- **WHEN** an inner payload is 61 containers deep and is submitted
- **THEN** it can be kept and flushed
- **WHEN** a 62-deep payload replaces an existing pending value
- **THEN** the turn fails with cause handler and the previous pending value and timer stay intact

### Requirement: Documentation and samples state the deadlines' limits

The guide SHALL describe latest-wins replacement, creation, public recovery, payload depth and sender/reply semantics.
It SHALL state that deadlines use handling time and the core's microsecond rounding and commit-time clamping, not
promised execution times. The contract SHALL be pinned; the sample service SHALL host the type without an instance.

#### Scenario: Paused past the deadline

- **WHEN** a debouncer's Flush becomes due while it is paused and it later resumes
- **THEN** it flushes when claimed, with no promise that maxWait bounded the pause
