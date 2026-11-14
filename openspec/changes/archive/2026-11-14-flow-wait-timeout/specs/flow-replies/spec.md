## MODIFIED Requirements

### Requirement: A flow accepts only the first eligible reply from its target

Running flows SHALL check known reply-enabled request, sender equality, absence of a saved reply and absence of a
closed wait before validating a value. Failing these checks SHALL succeed without state writes. A new eligible value
SHALL be retained once, within ADR 0044's bounds; NUL/depth above 61 SHALL fail permanently without state change.
Reply turns SHALL NOT run flow code or consult time to choose an outcome. An eligible reply SHALL close a supported
open single wait with or without a deadline, even after that deadline; joins remain reserved. Done/cancelled SHALL ignore Reply; cancelling remains unavailable in this piece.

#### Scenario: Ignored reply

- **WHEN** Reply names an unknown id or plain send, comes from another address, repeats a saved reply or names a closed wait
- **THEN** it succeeds without changing state/version or arming work, even if its value would fail the depth check
- **AND** a duplicate with a different value does not replace the first value

#### Scenario: Sender boundary

- **WHEN** a reply comes from direct HTTP, a child, a delegate or dead-letter redrive
- **THEN** it is ignored because its sender is absent or differs from the saved target
- **WHEN** a trusted remote worker completes the target's leased turn with a reply
- **THEN** its sender is the target and it passes the same sender check

#### Scenario: Missing and deep values

- **WHEN** a decoded eligible Reply has a 62- or 63-level value
- **THEN** it fails permanently and saves nothing
- **WHEN** its value field is absent
- **THEN** ordinary message decoding fails before reply handling
- **WHEN** its value is explicitly null
- **THEN** null is retained as present data

#### Scenario: Early reply

- **WHEN** an eligible reply arrives before its request has a wait
- **THEN** the value is retained and armed is unchanged
- **AND** unique replies to bounded sends cannot exceed the existing 2000-record bound

#### Scenario: Reply closes an open wait

- **WHEN** an eligible reply reaches a supported open single wait with or without a deadline
- **THEN** value, received outcome, cancellation of an older armed message and an immediate Continue commit together
- **AND** the Continue uses Flow::retry and the reply's correlation, with no flow code executed in the Reply turn

## ADDED Requirements

### Requirement: A single wait can time out as data

Step::wait_timeout SHALL return Waited<T>::Received(T) or TimedOut only from a committed wait decision. A new wait
SHALL save until=ceil(claim time + timeout), bounded by MAX_DELAY and MAX_INSTANT, with one request and mode all.
It SHALL check the time bounds before using an early reply. Replay SHALL use the saved until and ignore the newly
passed duration. An indefinite wait, join or different handle SHALL NOT match this API.

#### Scenario: New timed wait

- **WHEN** a new timed wait has no saved reply
- **THEN** it commits an open wait and an At(until) continuation atomically, suspending without returning data
- **AND** the deadline counts from that turn's claim, not the request's earlier send or wall time at the helper call

#### Scenario: Early reply and zero duration

- **WHEN** a new timed wait has an early reply
- **THEN** it commits received with a saved deadline and arms now, even for zero duration
- **WHEN** a zero-duration wait has no reply
- **THEN** it commits open first and cannot choose timeout in its creating turn

#### Scenario: Time limits

- **WHEN** the claim is on a store microsecond and duration is 1ns
- **THEN** the deadline is 1 microsecond later
- **WHEN** duration is MAX_DELAY and the result fits MAX_INSTANT
- **THEN** it is accepted
- **WHEN** duration exceeds MAX_DELAY by 1ns or its rounded result exceeds MAX_INSTANT
- **THEN** it fails permanently even if a reply is already saved
- **AND** at MAX_INSTANT minus 1s, 1s fits but 1s plus 1ns does not

#### Scenario: Replay matching

- **WHEN** a matching timed wait is replayed with another duration, even an invalid one
- **THEN** the saved deadline still decides
- **WHEN** indefinite and timed APIs are swapped, the handle differs, a request is reused, a join is encountered or entry 1001 is needed
- **THEN** the turn fails permanently without progress

### Requirement: A timeout decision commits before subsequent flow work

Only a flow-executing Continue with an open timed wait, no saved reply and claim time at/after until SHALL choose
timedOut. It SHALL save received=[] and arm now in one commit, returning no data to later flow code in that turn.
An early execution SHALL re-arm at the saved until without a new entry. A saved reply in an open wait SHALL instead
cause a committed received decision. The first latch and predecessor decode-error/panic rules SHALL apply.

#### Scenario: Timer execution and later replay

- **WHEN** an executing Continue reaches an unanswered wait at until
- **THEN** it commits timedOut and the next Continue without executing code after the wait
- **WHEN** that saved decision is replayed later
- **THEN** the helper returns TimedOut without decoding data or reconsidering the clock

#### Scenario: Early execution

- **WHEN** execution reaches the open wait before until, including after deliver-now or a public poke
- **THEN** it re-arms At(until) and keeps the same journal entry and deadline

#### Scenario: Supplied reply past the deadline

- **WHEN** supplied supported progress has an open timed wait with a stored reply and execution occurs after until
- **THEN** it commits received before a later turn decodes and returns that value

#### Scenario: Failed decision

- **WHEN** the attempted timeout decision does not commit
- **THEN** no timeout branch has been returned to flow code and the wait stays open
- **AND** a reply handled before a later successful decision may still choose received

#### Scenario: Latch and decoding

- **WHEN** flow code swallows a deciding suspension and calls another helper
- **THEN** only the first latch controls the turn and later helpers do nothing
- **WHEN** wait_timeout is never polled
- **THEN** it changes nothing
- **WHEN** a committed received result fails typed decoding or its deserializer panics and is caught
- **THEN** the returned flow fails permanently, later helpers are blocked and the saved outcome remains

### Requirement: Reply and timeout races follow actual mailbox handling order

A deadline SHALL be the earliest time timeout may be selected, not a reply acceptance cutoff. Reply turns SHALL
close eligible open single waits without reading the clock. Mailbox ordering SHALL remain (deliverAt,seq). Once a
choice commits it SHALL NOT change; later replies for a closed wait SHALL be ignored before value checks. Timeout
SHALL NOT cancel target work or perform cleanup.

#### Scenario: Equal delivery times

- **WHEN** an eligible reply and armed timer have the same deliverAt
- **THEN** the lower seq is handled first and its successful decision wins
- **AND** both sequence orders are supported without introducing a separate race rule

#### Scenario: Reply after deadline with no timer

- **WHEN** a timer was purged or cancelled and an eligible reply is handled after until while the wait remains open
- **THEN** received commits and arms now using that reply's correlation

#### Scenario: Late reply after timeout

- **WHEN** timedOut was committed and a reply arrives, including one exceeding the reply depth bound
- **THEN** it is ignored without state/version changes and the target's ongoing work is unaffected

#### Scenario: Later failure cannot change the branch

- **WHEN** timeout commits, a following step fails transiently, and a late reply arrives before retry
- **THEN** retry still returns TimedOut and does not reissue the request

#### Scenario: Delayed commit and pause

- **WHEN** a commit clamps a past At(until) or a pause postpones execution
- **THEN** the saved deadline is unchanged and actual mailbox handling still decides
- **AND** no hard real-time response cutoff or timeout completion time is promised

### Requirement: Timed waits retain continuation recovery and metadata

Public Continue SHALL preserve normalization before replay; before until it SHALL not shorten the deadline. A lost,
purged, cancelled or failed timer SHALL leave an open wait recoverable by reply or public Continue, without polling.
Timeout closure SHALL carry the executing Continue's correlation; reply closure SHALL use the reply's. Deadline
self-sends SHALL count as timers, and arm-now continuations as immediate sends under existing observer rules.

#### Scenario: Recovery after a missing timer

- **WHEN** the timer is lost and no reply arrives
- **THEN** the run stays open until a public Continue normalizes and resumes timeout processing
- **AND** before until it re-arms there, while at/after until it commits timeout in its own deciding turn

#### Scenario: Correlation and timer classification

- **WHEN** a wait arms At(until) then closes timedOut
- **THEN** the first send is a timer and the continuation after the decision is immediate
- **AND** timeout keeps the executing timer chain's correlation, while early-reply closure uses the closing Continue's correlation
