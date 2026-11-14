# flow-sends Specification

## Purpose
A flow step can send a message to an actor, creating it first if needed, in the same commit that saves the step and
arms the next run. A retry of a later step therefore never sends again: the message is queued once.

## Requirements

### Requirement: A new send step commits actor work with its journal entry

Step::send SHALL prepare one send and optional independent/child create without exposing Turn. The runner SHALL commit
those effects, one send journal entry and its next Continue atomically. A saved send SHALL replay without enqueueing
again. Sends SHALL be immediate, carry the executing message's correlation id and have no retry policy. This SHALL
promise enqueueing once per committed entry, not successful or exactly-once target execution.

#### Scenario: Create and send

- **WHEN** a new send creates an independent target and sends its first message
- **THEN** target creation, message enqueueing, send entry and next continuation commit together
- **AND** the entry records destination, generated message id, create kind and reply=false

#### Scenario: Later failure

- **WHEN** a later external step fails and the flow is retried
- **THEN** the send entry replays without another create or enqueue and the later step follows ordinary retry rules

#### Scenario: Metadata and target failure

- **WHEN** a send step commits under an executing Continue with correlation and a retry policy
- **THEN** the outgoing message carries that correlation and no retry policy, while the next Continue has Flow::retry()
- **AND** a target handler failure does not undo saved flow progress or automatically resend the message

### Requirement: Send replay invokes no argument conversions

Target SHALL hold borrowed data without serializing it. Step::send SHALL check latch, replay and room before converting
arguments. Replay SHALL match id, kind and reply=false, then advance without converting/serializing the target, message
or initial state. Target and create kind SHALL NOT be replay comparisons. Caller argument construction and Rust Drop
remain ordinary replayed code; their effects SHALL NOT be described as suppressed.

#### Scenario: Proposed arguments change after commit

- **WHEN** a committed send is replayed with different target/message/state arguments whose conversions would panic
- **THEN** replay returns Ok without conversion, keeps the saved destination and buffers no effects

#### Scenario: Latch and unpolled future

- **WHEN** an earlier helper latched an outcome, or a send future is created and dropped without polling
- **THEN** the send invokes no conversion or serialization and makes no new progress

#### Scenario: Replay misuse

- **WHEN** send encounters another id/kind or a reply-enabled send, an id is invalid/repeated, or it would append entry 1001
- **THEN** the turn fails permanently without new actor effects or progress

### Requirement: New send conversion is protected by the first latch

Before new target/message/state conversion, Step SHALL set InProgress for the send. Success SHALL replace it with
the new entry/outbox, and a returned conversion error with permanent failure, before returning Suspend. A caught panic
SHALL leave the marker, blocking later helpers and causing permanent failure when the flow returns. Uncaught panics
and flow code that never returns SHALL keep ordinary core semantics.

#### Scenario: Caught conversion panic

- **WHEN** target conversion/address serialization, message conversion/serialization or create-state serialization panics
  and flow code catches it then returns Ok or tries another helper
- **THEN** no later helper runs and the turn fails permanently without progress, naming the send

#### Scenario: Swallowed suspension

- **WHEN** flow code catches a new send's private suspension and returns another result
- **THEN** the first send's latch decides the turn, with one outbox and one new entry

### Requirement: Outbox failures preserve core atomicity and failure policy

The runner SHALL use the public raw create/send operations. Store errors SHALL retain their error chain and retryability;
Invalid/SendRejected SHALL fail permanently. A rejected commit SHALL save no outbox or progress. Initial state checks
SHALL apply even for an existing target. A send to the run itself SHALL be refused before buffering. Other create and
child-ownership rules SHALL remain the core's.

#### Scenario: Registry failure and invalid target

- **WHEN** registry lookup fails with a Store error
- **THEN** the executing Continue fails with cause node and its retry policy applies, with no progress
- **WHEN** the type is unregistered or does not accept the message
- **THEN** it fails permanently with cause refused and no progress

#### Scenario: Existing actor and child collision

- **WHEN** an independent create targets an existing actor with valid proposed state
- **THEN** that actor's existing state is reused untouched
- **WHEN** proposed state is null, contains NUL or is too deep
- **THEN** the turn fails permanently even if the target exists
- **WHEN** a child create collides with an actor owned elsewhere
- **THEN** the commit is rejected with no create, send or new entry

#### Scenario: Missing destination and recovery

- **WHEN** a send without create targets a missing actor
- **THEN** commit fails with cause commit and the saved run remains stalled
- **WHEN** the target is subsequently created and the run is poked
- **THEN** the uncommitted send can be attempted afresh

#### Scenario: Self target

- **WHEN** a new send names its own run address
- **THEN** it fails permanently and buffers nothing, preventing a new self-Continue or self-Reply from this helper

### Requirement: Reserved flow behavior is explicit during rollout

This release SHALL define the ADR 0044 wire shape but SHALL permanently refuse Reply, Cancel and execution of cancelling
state as unavailable features. Continue in done/cancelled SHALL do nothing. Wait and request-send entries SHALL not
replay through plain-send/step/sleep APIs. Docs SHALL require all hosts/workers of a type upgraded before later features
are enabled, since equal contract hashes do not establish equal supported behavior.

#### Scenario: Reserved message and phase

- **WHEN** Reply or Cancel reaches this release, or Continue reaches cancelling state
- **THEN** the message fails permanently with a later-release diagnostic and no effects
- **WHEN** Continue reaches structurally valid done/cancelled state
- **THEN** the state stays and no flow code runs

#### Scenario: Deployment choices

- **WHEN** a host adopts the changed flow contract
- **THEN** docs offer draining/archiving and deliberate registration reset, or a pinned old binary hosting old names
  alongside new binaries hosting new names with otherwise compatible contracts
- **AND** they do not promise automatic live migration, preservation of deleted mailboxes or a generic export/restore utility
