## MODIFIED Requirements

### Requirement: The public step API uses ordinary futures and errors

Flow::run SHALL accept `&mut Step` and owned Input and return a Send future of HandlerResult. Step::run SHALL accept
an id and a Send FnOnce(String) returning a Send future of Result<T, HandlerError>, where T is Serialize,
DeserializeOwned and Send. The String SHALL be the external idempotency key. Step SHALL expose no Turn. A new send helper SHALL buffer actor effects with its journal entry through the runner.

#### Scenario: A typed external step

- **WHEN** a flow awaits a step closure that returns Result<String, HandlerError> and propagates it with `?`
- **THEN** it compiles with ordinary async code and receives a String when that result is replayed

### Requirement: Run state has a stable journal representation

FlowState SHALL contain input, journal, status, armed, replies and cleanup. Defaults SHALL be empty journals/lists,
running and null armed. The ADR 0044 contract SHALL include step/sleep/send/wait entries and running/cancelling/done/
cancelled status from this release. FlowMsg SHALL include Continue, Reply and Cancel. Reply.value and stored reply
values SHALL be required but permit null. Each later behavior SHALL be enabled separately without implying feature support.

#### Scenario: Literal progress state

- **WHEN** a run has saved a unit step
- **THEN** its journal can contain `{"kind":"step","id":"notify","result":null}`
- **AND** it round-trips under the same schema that accepts `{"kind":"sleep","id":"wait","until":"2026-11-14T00:00:00Z"}`

#### Scenario: Old supported progress

- **WHEN** a supported previous-release step/sleep state omits replies and cleanup
- **THEN** it decodes with empty new lists and its ordinary steps/sleeps can replay
- **AND** existing historical reads are not subjected to a newly added result-depth check

#### Scenario: Journal wire forms

- **WHEN** an entry is encoded
- **THEN** step and sleep keep their current shapes, and send contains id, to, message, create and reply
- **AND** create is null/independent/child with default null, and reply defaults false
- **AND** wait contains id, requests, mode, until, outcome and received, with required all/first mode,
  null default deadline/outcome and empty default received; outcome is null/received/timedOut

#### Scenario: Required nullable values

- **WHEN** Reply.value or a stored reply value is null
- **THEN** it is accepted as present data
- **WHEN** the value field is absent
- **THEN** typed decoding refuses it and the generated schema describes it as required

#### Scenario: Schema bounds

- **WHEN** a host inspects a regenerated flow contract
- **THEN** each journal allows at most 1000 entries, replies at most 2000, wait requests 1 through 100 and received at most 100
- **AND** entry ids retain their existing pattern/length, statuses/enums have the documented names and cross-field
  validation is documented as a typed check rather than a JSON Schema promise

### Requirement: Journal and status invariants are validated

Typed decoding SHALL enforce ADR 0044's per-journal id/count, timestamp, status and cross-reference rules. Each journal
SHALL allow at most 1000 unique valid ids, and send message ids SHALL be unique across both. Replies SHALL be unique
and reference reply-enabled sends. Unknown fields SHALL remain ignored. Supplied progress SHALL remain trusted claims,
not proof that external or actor effects occurred; typed validity SHALL NOT imply the release implements every feature.

#### Scenario: Invalid state

- **WHEN** a state has duplicate/invalid entry ids, an overfull journal/reply list, duplicate send ids or a time after MAX_INSTANT
- **THEN** typed decoding refuses it

#### Scenario: Separate phases

- **WHEN** an id occurs once in main and once in cleanup
- **THEN** it is not a duplicate across those namespaces
- **WHEN** running has cleanup, done has cleanup/armed/an open main wait, or cancelled has armed/an open cleanup wait
- **THEN** decoding refuses it
- **AND** cancelling/cancelled may retain an interrupted open main wait without rewriting that journal

#### Scenario: Restored progress

- **WHEN** a host creates structurally valid supported saved progress with running status and no armed message
- **THEN** public Continue can recover it and replay its saved results without redoing those effects

## ADDED Requirements

### Requirement: Wait references are structurally valid before behavior is enabled

A wait SHALL reference unique earlier reply-enabled sends and be the only wait for each request in its journal.
An open wait SHALL be last. Cleanup MAY reference main requests not closed by a main wait. Cross-phase overlap SHALL
be permitted only between an interrupted open main wait and its cleanup wait while cancelling/cancelled. Other closed
wait reuse SHALL be refused. Stored reply ids SHALL be unique references to reply-enabled sends in either journal.

#### Scenario: Unknown or reused request

- **WHEN** a wait names an unknown/non-request/later send, repeats an id, follows an open wait, or reuses a request in its journal
- **THEN** decoding refuses the state

#### Scenario: Cleanup consumes an interrupted request

- **WHEN** the run is cancelling/cancelled, a main wait is still open and cleanup waits for one of its requests
- **THEN** the overlap is accepted, including after the cleanup wait closes
- **WHEN** the main wait was already closed
- **THEN** cleanup cannot wait on that request again

### Requirement: Saved wait outcomes have consistent reply selections

An open wait SHALL have empty received. A closed wait's received SHALL be a duplicate-free subset in request order,
with stored reply values. Received/all SHALL include every request and received/first exactly one. TimedOut SHALL
require a deadline; timedOut/all SHALL include a strict subset and timedOut/first none. These are structural rules;
this release SHALL NOT yet execute waits.

#### Scenario: Invalid outcome

- **WHEN** an open wait selects replies, a selected value is absent, selections are repeated/out of order, all/first
  cardinality is wrong, or timedOut has no deadline
- **THEN** typed decoding refuses the state

#### Scenario: Partial timeout and interrupted main wait

- **WHEN** an all wait with a deadline times out with a strict subset of stored replies in request order
- **THEN** its closed outcome is structurally valid
- **AND** a cleanup wait can record that outcome while its interrupted main wait remains open under cancelling/cancelled
