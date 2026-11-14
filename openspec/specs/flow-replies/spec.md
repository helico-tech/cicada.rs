# flow-replies Specification

## Purpose
A flow step can send a typed request to an actor and wait for its reply without polling: the run parks with an open
wait, and the first reply from the request's target closes it and lets the flow go on with the value.

## Requirements

### Requirement: A typed request commits a reply route with actor work

Step::request SHALL return a typed Request<T> on replay and commit a new reply-enabled send through the atomic outbox.
The builder SHALL receive ReplyTo<T> containing the run address and the generated outgoing message id. Request SHALL
have a private constructor and expose its id. ReplyTo SHALL encode required run and request fields, ignore unknown
fields and have the same schema for every T. Its type SHALL connect the target's reply type to the waiting flow.

#### Scenario: Typed creation and send

- **WHEN** a flow requests work using an existing target or Target::create/child and a builder taking ReplyTo<JobResult>
- **THEN** the request's type is inferred and the flow future is Send
- **AND** target effects, the reply-enabled send entry and next Continue commit together with the same generated id

#### Scenario: Replay and first latch

- **WHEN** a saved request replays, another helper already latched, or a request future is never polled
- **THEN** no target conversion, builder invocation or message/state encoding occurs
- **AND** replay returns the saved request id without another send

#### Scenario: Request misuse and panic

- **WHEN** a request meets a plain send, another id/kind, an invalid/repeated id, entry 1001 or a self target
- **THEN** it latches permanent failure without progress
- **WHEN** new user conversion, builder or serialization panics and flow code catches it
- **THEN** the InProgress marker blocks later helpers and fails the returned flow permanently

#### Scenario: Rejected request commit

- **WHEN** the request's child create or send rejects the commit
- **THEN** no send entry, new request id or partial effects survive
- **AND** recovery can attempt the uncommitted request with a fresh id

### Requirement: Targets send replies through ordinary actor effects

ReplyTo::send SHALL encode and validate the value synchronously and return a Send future borrowing only the Turn.
When polled it SHALL send Reply with the route's request id, current correlation, immediate delivery, no retry policy
and ordinary retention/observer headers. Encoding, NUL and depth above 61 SHALL return TurnError::Invalid without
buffering. Raw-send errors SHALL retain their ordinary variants. The helper SHALL introduce no external sender authority.

#### Scenario: Owned encoded value

- **WHEN** a target builds the helper future then drops its value and ReplyTo
- **THEN** the future can still be awaited and sends the captured encoded value
- **AND** encoding does not require T to be Sync merely to make this future Send

#### Scenario: Value boundaries

- **WHEN** the target replies with null or a value 61 containers deep
- **THEN** the helper accepts it
- **WHEN** encoding fails, a NUL occurs or the value is 62 containers deep
- **THEN** it returns Invalid and buffers no reply

#### Scenario: Deleted run

- **WHEN** the target buffers a reply to a run that was deleted
- **THEN** ordinary commit rejection can discard that target turn's state and all effects
- **AND** the helper does not promise a delivery receipt or automatic recovery

### Requirement: A flow accepts only the first eligible reply from its target

Running and cancelling flows SHALL check known reply-enabled request, sender equality, absence of a saved reply and absence of any
closed wait in either journal before validating a value. Failing these checks SHALL succeed without state writes. A new eligible value
SHALL be retained once, within ADR 0044's bounds; NUL/depth above 61 SHALL fail permanently without state change.
Reply turns SHALL NOT run flow code or consult time to choose an outcome. An eligible reply SHALL close a supported
open wait with or without a deadline when sufficient replies are present, even after that deadline. Partial all
replies SHALL keep armed unchanged. Done/cancelled SHALL ignore Reply; cancelling SHALL route only to the active cleanup wait without changing main progress.

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

- **WHEN** an eligible reply arrives before its request has an active-phase wait
- **THEN** the value is retained and armed is unchanged, including a main request retained during cancelling
- **AND** unique replies to bounded sends cannot exceed the existing 2000-record bound

#### Scenario: Reply closes an open wait

- **WHEN** an eligible reply reaches a supported open wait with or without a deadline and makes it sufficient
- **THEN** value, received outcome, cancellation of an older armed message and an immediate Continue commit together
- **AND** the Continue uses Flow::retry and the reply's correlation, with no flow code executed in the Reply turn

### Requirement: A single wait commits its outcome before returning data

Step::wait SHALL accept a borrowed Request<T> and return T only by replaying a committed received wait. New waits SHALL
reference an unused earlier request in the active journal, or during cleanup a main request not covered by a closed
main wait; use mode all, one request and no deadline. A new
wait or closure SHALL suspend even if the value is already present. An open wait SHALL park without polling, with
armed=null. The first latch SHALL control the returned flow regardless of swallowed suspension or later helper calls.

#### Scenario: Wait before reply

- **WHEN** a new wait has no saved reply
- **THEN** it commits one open entry, clears armed and completes the current turn without another continuation
- **AND** no handler remains open waiting for the target

#### Scenario: Reply before wait

- **WHEN** a new wait finds an early reply
- **THEN** it commits a closed received entry and arms now, without returning the value to later flow code in that turn
- **AND** a later replay returns the value, using the closing Continue's correlation rather than a stored reply correlation

#### Scenario: Supplied open wait with a reply

- **WHEN** supported supplied progress has an open wait and an already saved matching reply
- **THEN** replay commits closure and arms before returning the value in a later execution

#### Scenario: Wait misuse

- **WHEN** wait sees another handle, id or kind, a saved multi-request/first/deadline wait, a reused request, an unknown request or entry 1001
- **THEN** it latches permanent failure without new progress
- **AND** request/wait ids share the phase journal's ordinary id and capacity rules

#### Scenario: Typed decode failure

- **WHEN** a closed received value cannot decode as T
- **THEN** the turn fails permanently and the saved outcome remains available after code is fixed and Continue sent
- **WHEN** custom deserialization panics and flow code catches it
- **THEN** an InProgress marker blocks later work and makes the returned flow fail permanently

#### Scenario: One-request all structural equivalence

- **WHEN** wait and wait_all with one handle replay the same id and ordered request
- **THEN** both accept the same all/no-deadline decision, returning their respective result containers
- **AND** the timed pair behaves likewise with a present deadline, while mode first never matches all

### Requirement: Parked and closed waits preserve normal continuation recovery

Public Continue SHALL retain its existing normalization and retry policy. Its later execution SHALL park an open
wait without appending entries, or replay a closed wait's saved value. Pauses, cancellations or purges of continuations
SHALL NOT discard progress. No automatic watch or reply SHALL be synthesized for target failure or deletion.

#### Scenario: Public Continue while parked

- **WHEN** a public Continue reaches a parked wait without a reply
- **THEN** it normalizes to an armed continuation and that continuation parks again
- **AND** no wait entry is added and no periodic polling message is sent

#### Scenario: Lost continuation after closure

- **WHEN** a closing continuation is purged or cancelled and the host sends public Continue
- **THEN** the normal continuation replays the saved reply without reissuing the request

#### Scenario: Pause and target failure

- **WHEN** a run is paused while replies arrive
- **THEN** they wait in its mailbox and are processed after resume
- **WHEN** a target fails without replying or disappears
- **THEN** an indefinite wait can stay parked until an eligible reply or later supported intervention

### Requirement: Request support preserves the reserved flow contract

Flow contracts and existing ready-made actor contracts SHALL remain equal to the predecessor's generated contracts.
ReplyTo SHALL have pinned required address/id fields and a type-independent schema. Docs SHALL require all hosts and
workers upgraded before enabling requests/waits; host targets adding ReplyTo SHALL follow normal strict-hash upgrades.

#### Scenario: Mixed behavior versions

- **WHEN** a predecessor host handles Reply or replays a request despite matching the flow hash
- **THEN** it can permanently refuse that behavior
- **AND** docs explain that a refused Reply is not repaired by dead-letter redrive because the sender changes

#### Scenario: Stable schema

- **WHEN** contracts are regenerated after this implementation
- **THEN** existing flow and ready-made actor schemas/hashes have no diff
- **AND** ReplyTo<String> and ReplyTo<u64> generate the same route schema, while a new target fixture exposes the route

### Requirement: A single wait can time out as data

Step::wait_timeout SHALL return Waited<T>::Received(T) or TimedOut only from a committed wait decision. A new wait
SHALL save until=ceil(claim time + timeout), bounded by MAX_DELAY and MAX_INSTANT, with one request and mode all.
It SHALL check the time bounds before using an early reply. Replay SHALL use the saved until and ignore the newly
passed duration. An indefinite, multi-request, first-mode or different-handle wait SHALL NOT match this API; a one-request all
wait written by a join helper SHALL match.

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
- **WHEN** indefinite and timed APIs are swapped, the handle differs, a request is reused, a multi-request or first-mode wait is encountered or entry 1001 is needed
- **THEN** the turn fails permanently without progress

### Requirement: A timeout decision commits before subsequent flow work

Only a flow-executing Continue with an open timed wait, insufficient saved replies and claim time at/after until SHALL choose
timedOut. It SHALL save the timeout selection (empty for single/first, the available subset in request order for all) and arm
now in one commit, returning no data to later flow code in that turn.
An early execution SHALL re-arm at the saved until without a new entry. Sufficient saved replies in an open wait SHALL instead
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
close eligible open waits when enough replies are present, without reading the clock. Mailbox ordering SHALL remain (deliverAt,seq). Once a
choice commits it SHALL NOT change; later replies for a closed wait SHALL be ignored before value checks. Timeout
SHALL NOT cancel target work or perform cleanup.

#### Scenario: Equal delivery times

- **WHEN** an eligible reply that makes the wait sufficient and the armed timer have the same deliverAt
- **THEN** the lower seq is handled first and its successful decision wins
- **AND** both sequence orders are supported without introducing a separate race rule

#### Scenario: Reply after deadline with no timer

- **WHEN** a timer was purged or cancelled and an eligible reply making the wait sufficient is handled after until while the wait remains open
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

### Requirement: Typed joins return ordered all results or a stable winner

Step SHALL provide wait_all, wait_all_timeout, wait_first and wait_first_timeout over a borrowed slice of homogeneous
Request<T>, with T:DeserializeOwned+Send and no T:Sync requirement. All SHALL return Vec<T>; timed all SHALL return
Vec<Waited<T>> with one slot per input. First SHALL return (input index,T); timed first SHALL return Waited<(index,T)>.
The helpers SHALL reuse the existing journal and wait protocol, without exposing Turn or running concurrent steps.

#### Scenario: Partial typed results

- **WHEN** a timed all wait over [a,b,c] commits timeout with selected replies [a,c]
- **THEN** replay returns [Received(a),TimedOut,Received(c)] regardless of a/c arrival order
- **AND** at least one TimedOut slot indicates timeout; a received outcome has only Received slots

#### Scenario: First result

- **WHEN** a first wait over [a,b] selects b
- **THEN** replay returns index 1 and b's decoded value, wrapped in Received for a timed first wait
- **WHEN** a first wait times out
- **THEN** its timed helper returns TimedOut without a value or winner index

#### Scenario: Send future and borrowed handles

- **WHEN** a flow passes a borrowed Vec or array of Request<T> where T is Send but not Sync
- **THEN** its helper future remains Send and the helper invokes no iterator or conversion code

### Requirement: Join inputs and replay matching preserve request positions

New waits SHALL require 1..100 distinct known reply-enabled requests unused by another wait in the phase: earlier active-journal requests, or
during cleanup main requests not covered by a closed main wait,
a valid id and journal room. Timed bounds SHALL be checked before early replies. Replay SHALL match the exact ordered
requests, mode and deadline presence, ignoring a new duration. A one-request all wait SHALL be structurally equivalent
to the corresponding single wait. A first wait SHALL remain a different mode even with one request.

#### Scenario: Invalid list or reuse

- **WHEN** the list is empty, has 101 handles, duplicates, unknown requests or requests already in another wait
- **THEN** the turn fails permanently without progress
- **AND** a losing request of a closed first wait cannot be reused

#### Scenario: Order changed on replay

- **WHEN** replay reorders, extends or replaces the saved list, or switches all and first mode
- **THEN** it fails permanently and keeps the saved decision
- **AND** the helper does not sort or deduplicate the list

#### Scenario: Capacity and deadline

- **WHEN** 100 requests are joined as journal entry 1000
- **THEN** one join entry is accepted
- **WHEN** entry 1001 is needed or a new deadline exceeds the time limits even with enough early replies
- **THEN** the turn fails permanently
- **WHEN** a matching saved timed join is replayed with an invalid new duration
- **THEN** the saved deadline is used without validating that new duration

### Requirement: First waits select the earliest accepted listed reply

The replies array SHALL preserve acceptance order. A first wait with multiple saved listed replies SHALL select the
one appearing earliest in that array, not the first in the request list. Supplied progress SHALL be trusted including
array order. All selection SHALL follow request order. A committed selection SHALL never be recomputed from later
replies or the clock, and only selected values SHALL be decoded.

#### Scenario: Multiple early replies

- **WHEN** b is accepted before a and both are saved before wait_first([a,b])
- **THEN** the deciding turn saves [b] and later replay returns index 1
- **AND** reversing acceptance order selects a, even though the request list is unchanged

#### Scenario: Supplied open first wait

- **WHEN** supplied open first-wait progress contains multiple listed replies
- **THEN** it commits the earliest in the stored replies array before exposing any data

#### Scenario: Decode selection and failure

- **WHEN** an all result's second selected value fails typed decoding
- **THEN** the turn fails permanently and the committed decision remains for recovery after fixing code
- **WHEN** a selected value's deserializer panics and flow code catches it
- **THEN** InProgress blocks later helpers and makes the returned flow fail permanently
- **AND** first losers and missing timed-all positions are never decoded

### Requirement: Partial join replies preserve the current wait until a decision

A new eligible reply to an active open all wait SHALL retain armed unchanged until all requests have replies. A
sufficient all/first wait SHALL close received, cancel older armed work and arm now using the completing reply's
correlation. A reply unrelated to that wait SHALL retain only its value. Reply turns SHALL run no flow code or clock
checks. State, outcome selection and continuation effects SHALL commit atomically.

#### Scenario: Partial and final all reply

- **WHEN** the first of two required replies arrives at a timed all wait
- **THEN** its timer id, deadline and timer correlation stay unchanged
- **WHEN** the last missing reply arrives
- **THEN** the received selection is in request order, the timer is cancelled and the next Continue uses that reply's correlation

#### Scenario: Parked or unrelated reply

- **WHEN** an incomplete untimed all wait accepts a partial reply
- **THEN** armed remains null
- **WHEN** an early reply names a request outside the active wait
- **THEN** that wait and its armed work are unchanged

### Requirement: Joined decisions commit before flow code consumes partial or complete data

New joins with enough replies SHALL commit received; otherwise they SHALL commit open and park or arm At(until).
A new zero-timeout incomplete join SHALL still commit open first. Replayed open joins SHALL select sufficient replies
before consulting time; due incomplete timed joins SHALL select timedOut with the current strict subset for all or
none for first. Every deciding path SHALL suspend until committed replay; inherited first-latch/recovery rules apply.

#### Scenario: Zero timeout and supplied progress

- **WHEN** a new zero-timeout all wait has only a partial set of replies
- **THEN** it commits open and arms, then a later deciding turn may commit timedOut with that subset
- **WHEN** supplied open progress past its deadline has all needed replies
- **THEN** it closes received instead of timedOut
- **WHEN** only a strict subset is present
- **THEN** it closes timedOut with that subset in request order

#### Scenario: Races and immutable replay

- **WHEN** the last required reply and timer have equal deliverAt
- **THEN** their seq order decides which successful outcome commits, in either order
- **WHEN** a decision commits, a later step fails, a late reply arrives and the flow retries
- **THEN** it returns the same selection/timeout slots without resending requests

#### Scenario: Lost timer and pause

- **WHEN** the timer is purged or the run paused
- **THEN** the predecessor's public Continue recovery and mailbox ordering remain in force
- **AND** joins do not add periodic polling or extend their saved deadlines

### Requirement: Closing a join closes all its requests without stopping targets

A closed join SHALL reject further replies for every listed request before value checks, including first losers and
missing timeout positions. Previously retained early loser values SHALL remain within existing bounds. The join SHALL
NOT cancel target work or perform cleanup. Closed main requests SHALL remain unavailable for later cleanup waits.

#### Scenario: Late loser and cleanup

- **WHEN** a losing first request later replies, even with an otherwise invalid deep value
- **THEN** state/version stay unchanged and the target is not automatically stopped
- **AND** a later finalizer must issue new cleanup work rather than wait again on that closed request

#### Scenario: Rollout and contract equality

- **WHEN** existing flow and actor contracts are regenerated after join support
- **THEN** they match the landed timeout predecessor without new wire fields or decoding restrictions
- **AND** docs require upgrading every host/worker before real joins, while acknowledging predecessors already handle one-request all waits
