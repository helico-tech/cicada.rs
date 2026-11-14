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

Running flows SHALL check known reply-enabled request, sender equality, absence of a saved reply and absence of a
closed wait before validating a value. Failing these checks SHALL succeed without state writes. A new eligible value
SHALL be retained once, within ADR 0044's bounds; NUL/depth above 61 SHALL fail permanently without state change.
Reply turns SHALL NOT run flow code. Done/cancelled SHALL ignore Reply; cancelling remains unavailable in this piece.

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

- **WHEN** an eligible reply reaches a supported open single wait
- **THEN** value, received outcome, cancellation of an older armed message and an immediate Continue commit together
- **AND** the Continue uses Flow::retry and the reply's correlation, with no flow code executed in the Reply turn

### Requirement: A single wait commits its outcome before returning data

Step::wait SHALL accept a borrowed Request<T> and return T only by replaying a committed received wait. New waits SHALL
reference an earlier unused reply-enabled send in the same journal, use mode all, one request and no deadline. A new
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

- **WHEN** wait sees another handle, id or kind, a saved join/deadline, a reused request, an unknown request or entry 1001
- **THEN** it latches permanent failure without new progress
- **AND** request/wait ids share the phase journal's ordinary id and capacity rules

#### Scenario: Typed decode failure

- **WHEN** a closed received value cannot decode as T
- **THEN** the turn fails permanently and the saved outcome remains available after code is fixed and Continue sent
- **WHEN** custom deserialization panics and flow code catches it
- **THEN** an InProgress marker blocks later work and makes the returned flow fail permanently

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
