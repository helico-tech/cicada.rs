# heartbeat-watch Specification

## Purpose
The heartbeat watch, `cicada.heartbeat-watch`, raises an alarm when an outside thing goes quiet: each `Beat` restarts
the allowed silence, and when it runs out the watch sends its stored alarm once and becomes silent until the next
`Beat`. `Rearm` restores a lost timer at the same deadline without counting as a sign of life.

## Requirements

### Requirement: A heartbeat watch has fixed settings and validated round state

The heartbeat-watch actor SHALL use fixed silence, alarm target/message and optional target retry settings. Silence
SHALL be positive and at most MAX_DELAY. Typed construction/decoding SHALL validate status/timestamp/armed consistency
and representable deadlines at or before MAX_INSTANT. Minimal state SHALL be waiting, with no timer. Existing core
depth/NUL checks SHALL apply; this actor SHALL add no core exception.

#### Scenario: Minimal and inconsistent states

- **WHEN** silence, target and alarm are supplied without optional fields
- **THEN** status defaults waiting and retry, lastBeatAt and armed default null; unknown fields are ignored
- **AND** typed decoding refuses waiting with timestamp/armed, alive without both, silent without timestamp or with
  armed, zero/too-long silence, and an overflowing or too-late deadline

#### Scenario: Schema and decoder boundaries

- **WHEN** a node that does not host the actor accepts shape-valid raw state
- **THEN** cross-field validity is not promised by JSON Schema alone
- **AND** a state that fails the hosting decoder later fails ordinary handling with cause state

#### Scenario: State depth

- **WHEN** an alarm payload is depth62 or depth63 in a new watch state
- **THEN** depth62 fits the state and can be sent, while depth63 is refused by ordinary create depth checks
- **AND** strings/keys containing NUL remain subject to the existing core refusal

### Requirement: A Beat starts or replaces one silence round atomically

Beat SHALL use turn claim time as lastBeatAt, set alive and replace any armed timer with a self-sent Expire at the
absolute deadline. Cancellation, timer send and state update SHALL commit together. Beat SHALL be usable as a
synthetic initial start without adding a Start message. The actor SHALL send no recovery notification.

#### Scenario: Start without an external first beat

- **WHEN** a host creates a waiting watch and sends Beat, including create-plus-send in one turn
- **THEN** handling Beat arms the silence timer and reports alive even before real external activity occurs
- **AND** a repeated create leaves existing settings and progress unchanged

#### Scenario: Continuing activity and recovery

- **WHEN** Beats are handled before their predecessor timer
- **THEN** each cancels/replaces that timer and no alarm is sent
- **WHEN** a Beat is handled after silent
- **THEN** a new round starts with no recovery message, and its later silence can send one new alarm

#### Scenario: Repeated envelopes

- **WHEN** a new Beat envelope is handled
- **THEN** it resets the deadline even if it represents the same external activity
- **AND** reuse of a message id follows only the core's ordinary dedup retention rules

### Requirement: Only the armed self timer can expire a round

Expire SHALL act only in alive when its sender is the watch and its message id equals armed. Other Expire messages
SHALL succeed without state writes. A valid Expire before the stored deadline SHALL replace its timer at that same
deadline without an alarm. At or after the deadline it SHALL enqueue the alarm and become silent atomically.

#### Scenario: Silence produces one alarm

- **WHEN** a valid due Expire commits
- **THEN** one alarm is enqueued, status becomes silent, armed becomes null and lastBeatAt stays
- **AND** no further silence alarms or timers occur until another Beat starts a round

#### Scenario: Forged, stale and copied timers

- **WHEN** Expire is external, from another actor, superseded, operator-retried under a new id, or redriven by dead letters
- **THEN** it changes neither state/version nor outgoing messages
- **AND** ordinary retries of the original armed message retain eligibility through their original id/sender

#### Scenario: Early administrative delivery

- **WHEN** deliver-now makes the armed Expire runnable before lastBeatAt plus silence
- **THEN** it sends no alarm and arms a replacement at the unchanged deadline

### Requirement: Rearm recovers the existing round without a new Beat

Public Rearm SHALL replace the alive round's timer at its unchanged deadline and preserve lastBeatAt. It SHALL do
nothing in waiting or silent, without writing state. Rearm SHALL be the supported recovery for lost or terminally
failed timers; it SHALL NOT fabricate a fresh beat or send a second alarm for a silent round.

#### Scenario: Missing timer

- **WHEN** purge or pending cancel removes the timer
- **THEN** state stays alive with its stale armed id and no watch failure notification is promised
- **WHEN** Rearm is handled
- **THEN** a replacement is armed at the old deadline, due immediately if that deadline is past

#### Scenario: Recovering a failed timer

- **WHEN** a timer failed permanently or exhausted retries and its target problem is fixed
- **THEN** Rearm can recover the same silence round with a new armed id
- **AND** retry/redrive copies of the failed timer remain ineligible

#### Scenario: No active round

- **WHEN** Rearm is handled in waiting or silent
- **THEN** state/version and outbox remain unchanged, even if the configured target equals self

### Requirement: Timing is based on handling and mailbox order

The deadline SHALL be last handled Beat claim time plus silence, without millisecond truncation. Absolute delivery
SHALL follow core microsecond ceiling and commit-time clamping. No alarm SHALL be enqueued before that measured
deadline. This SHALL NOT promise a maximum alarm latency or knowledge of external event time. Mailbox order SHALL
determine the outcome of a Beat and Expire race.

#### Scenario: Fractional duration and bounds

- **WHEN** silence includes a sub-microsecond fraction
- **THEN** the timer is rounded up by core delivery resolution, never earlier
- **WHEN** a Beat would make the deadline unrepresentable or later than MAX_INSTANT
- **THEN** it fails permanently with no round/timer effects committed

#### Scenario: Equal delivery timestamps

- **WHEN** Beat and Expire have equal deliverAt
- **THEN** lower seq runs first: Beat first replaces the timer with no alarm, Expire first alarms before Beat starts anew

#### Scenario: Pause, slow handling and restart

- **WHEN** a paused or slow watch resumes with a Beat ordered before its timer
- **THEN** that Beat replaces the timer using its handling claim time
- **WHEN** the timer is ordered first, including a Beat accepted after its deadline during a pause
- **THEN** the alarm can precede that Beat; external activity is not evidence of earlier mailbox handling
- **AND** node restart preserves durable progress while actor deletion removes its state and pending timer

### Requirement: Alarm effects and failures follow ordinary turn semantics

Active Beat/Rearm and eligible Expire SHALL refuse a target equal to self. Due Expire SHALL send the stored alarm
with watch sender, fresh id, configured target retry, no correlation and no explicit headers; ordinary observer
headers SHALL still apply. Alarm send and silent transition SHALL share one commit. Target processing SHALL remain
independent and at least once; the watch SHALL promise only one committed enqueue per round.

#### Scenario: Self target and no-op priority

- **WHEN** Beat, alive Rearm or an eligible Expire has target equal to this watch
- **THEN** the action is permanently refused with no state/cancel/send effects committed
- **AND** a forged Expire or inactive Rearm is still a no-op

#### Scenario: Target checks and retry policies

- **WHEN** the alarm type is unregistered or does not accept its message name
- **THEN** the timer is permanently refused; target existence is checked at commit
- **WHEN** the target is absent
- **THEN** the commit fails and the round stays alive with its previous armed id
- **WHEN** a registry read fails
- **THEN** its store error retains the retryable node cause and the timer's shared seven-attempt 1s..32s policy applies
- **AND** malformed target payload can fail later at the target; target retry never re-enters the watch

#### Scenario: Failure and uncertain commit

- **WHEN** a watch turn is known not to have committed
- **THEN** its alarm and state/timer effects are not partially applied
- **WHEN** commit acknowledgement is uncertain or a claim is abandoned
- **THEN** ordinary fenced core recovery applies; uncertainty is not described as rollback or permission for a second alarm

#### Scenario: Metadata and observation

- **WHEN** the alarm is enqueued
- **THEN** it has the watch as from, a new id, stored target retry and no correlation id copied from Beat/Rearm
- **AND** an installed observer can add ordinary send headers, while self timers use the existing timer-header selection
- **AND** terminal failures use ordinary watch/dead-letter behavior; a successful silence sends no extra recovery or stop notice
