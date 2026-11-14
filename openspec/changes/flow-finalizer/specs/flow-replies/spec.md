## MODIFIED Requirements

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

