# flows Specification

## Purpose
A durable flow is async code written as plain steps: on each execution it replays the saved results of earlier
steps, runs at most one new step, and saves its result together with the message that continues the run.

## Requirements

### Requirement: A flow is hosted as an ordinary actor

Flow SHALL supply a host-chosen TYPE, a serializable/schema-described Input, an async run method returning HandlerResult,
and retry() defaulting to the crate's shared policy. FlowActor<F> SHALL implement Actor with FlowRun<F> as ref and
FlowState<F::Input> as state. Ordinary App hosting and explicit create-and-Continue SHALL suffice; no core hook is added.

#### Scenario: Host start

- **WHEN** a host registers FlowActor, creates a typed run with FlowState::new(input), and sends Continue
- **THEN** it starts using ordinary actor operations, also when create-and-send occur in one producer turn

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

### Requirement: Saved steps replay in order

Each execution SHALL start with cursor zero. A saved Step call SHALL match the next journal entry's id and kind,
decode its result as T and advance without executing its closure. Duplicate call ids, mismatches or decode errors
SHALL fail permanently. A normal return with unconsumed saved entries SHALL fail permanently rather than mark done.

#### Scenario: Retry a later step

- **WHEN** two steps are committed and the third fails once
- **THEN** retry returns the first two saved results without running their closures and executes only the third again

#### Scenario: Changed replay

- **WHEN** flow code changes an expected id/kind, asks for a different result type, repeats an id, or returns Ok early
- **THEN** it fails permanently without committing new progress

### Requirement: One successful new step commits with the next continuation

The first unsaved call SHALL invoke its closure and encode its successful result. It SHALL suspend so the runner can
commit one new journal entry and one next armed Continue together. No second unsaved closure SHALL execute in that
turn. Appending beyond 1000 entries SHALL fail permanently. No saved progress SHALL commit when its turn fails.

#### Scenario: Two steps and completion

- **WHEN** a run executes two successful steps
- **THEN** separate execution turns commit each result and its next continuation
- **AND** a later replay consumes both results and marks the run done

#### Scenario: Lost progress before commit

- **WHEN** an external call succeeds but its turn is interrupted before committing
- **THEN** no result is saved, and a later attempt may call it again with the same idempotency key

#### Scenario: Step count limit

- **WHEN** a flow with 1000 saved entries asks for a new step
- **THEN** it fails permanently without invoking that new closure

### Requirement: The first latched outcome controls a returned flow

Step SHALL latch a new entry, the original error, or permanent validation error before returning private suspension.
Later Step calls SHALL invoke no closure. After Flow::run returns, the latch SHALL override its return. Without a latch,
an error SHALL fail normally and Ok SHALL complete only after consuming saved entries. The latch SHALL NOT preempt code.

#### Scenario: Swallowed suspension

- **WHEN** a flow catches the first new step's error, tries another step and returns Ok or another error
- **THEN** only the first closure ran and only its result and continuation commit

#### Scenario: Swallowed step failure

- **WHEN** a closure returns a transient or permanent HandlerError and flow code catches it and returns Ok
- **THEN** the original error still fails the turn with its original retryability and no new progress commits

#### Scenario: Business error as data

- **WHEN** a step successfully returns a serializable Result<T, String> value containing Err
- **THEN** that business outcome is saved and can be used for branching on replay

#### Scenario: Misuse that never returns

- **WHEN** flow code swallows suspension then hangs or panics
- **THEN** ordinary core timeout/panic semantics apply and the latched new result does not commit

### Requirement: Abandoned step futures cannot commit or start another step

Before invoking an unsaved closure, Step SHALL set InProgress(id), retained through awaiting, encoding and validation.
Its replacement by New/Failed/Permanent SHALL contain no further await. A leftover InProgress after Flow::run returns
SHALL fail permanently regardless of its return, preserving journal and armed and sending no continuation.
Further Step calls SHALL invoke no closure. An unpolled future SHALL change nothing.

#### Scenario: External timeout of a step

- **WHEN** flow code times out Step::run while its closure awaits and then returns Ok or a transient error
- **THEN** the turn fails permanently naming the abandoned id, with no journal or continuation changes

#### Scenario: Select cancellation followed by another step

- **WHEN** select cancels a polled Step::run, then flow code calls another step
- **THEN** the second closure is not invoked and the returned flow fails permanently

#### Scenario: A timeout inside the closure

- **WHEN** the closure itself returns a transient timeout error
- **THEN** its original error follows the normal flow retry policy

#### Scenario: An unpolled future

- **WHEN** flow code creates and drops a Step::run future without polling it, then returns Ok after consuming saved entries
- **THEN** no closure is invoked and the flow completes normally

#### Scenario: A caught step panic

- **WHEN** user code catches a panic from closure invocation, its future, or result serialization and returns Ok
- **THEN** the InProgress marker makes the turn fail permanently with no progress

### Requirement: New result values are validated before saving

Results SHALL encode as JSON, including null, contain no NUL in strings/keys and have at most 61 nested containers.
Encoding/validation failure SHALL be permanent with no new entry or continuation committed. The implementation SHALL
use a bounded walk on the public JSON value, not private core APIs. Input SHALL remain subject to the core state limit.

#### Scenario: Result boundaries

- **WHEN** a step returns null or a value 61 containers deep
- **THEN** it can be saved and replayed
- **WHEN** it cannot serialize, contains NUL, or is 62 containers deep
- **THEN** it fails permanently with earlier committed entries unchanged

### Requirement: Public Continue normalizes starts and recovery

If running and the current message id is not armed, Continue SHALL cancel the old armed id if present, self-send a
new Continue due now with Flow::retry() and the incoming correlation id, and save its id without executing flow code.
Only the armed message SHALL execute flow code. Every subsequent self-Continue SHALL use Flow::retry() and preserve
correlation. An operation SHALL never cancel its current message id.

#### Scenario: Initial retry policy

- **WHEN** a host starts a run with a Continue that has no envelope retry policy
- **THEN** that turn arms a self-Continue, and the executing step turn has the flow's retry policy

#### Scenario: Multiple public pokes

- **WHEN** several public Continues are queued for the same running instance
- **THEN** each unarmed one replaces the armed continuation without running flow code
- **AND** only the currently armed message can make the next step's progress

#### Scenario: Retrying the armed execution

- **WHEN** an armed step fails transiently and its envelope policy retries
- **THEN** it keeps its id and replays normally, without a new normalization turn

### Requirement: Completed and stalled runs use ordinary controls

Completion SHALL set status done and armed null; a done run SHALL ignore Continue and remain until deletion.
Cancel, purge or exhausted retries of a continuation SHALL leave saved progress available; another public Continue
SHALL recover it, resetting attempts by arming a fresh message. Ordinary watches, pause, resume, delete and claim-limit
semantics SHALL apply. The runner SHALL install no watch of its own.

#### Scenario: Completed run

- **WHEN** a completed run receives Continue again
- **THEN** no closure or continuation runs and its saved results remain readable

#### Scenario: Stalled continuation

- **WHEN** the armed message is cancelled, purged or fails terminally, then a public Continue is sent
- **THEN** a fresh armed message resumes from saved progress using the flow's policy

#### Scenario: Out of retries

- **WHEN** an armed step exhausts its retries
- **THEN** its message ends failed, core watchers are notified, and earlier saved entries remain

### Requirement: Step keys and replay obligations are explicit

Each closure SHALL receive `<canonical run address>/<step id>` unchanged across attempts and recovery. Run keys SHALL
not be reused for different jobs, including after deletion. The guide SHALL state that external execution is at least
once, code outside steps replays and must be deterministic/free of external effects, and incompatible flow code needs
old runs drained or a new type name even if its schema hash did not change.

#### Scenario: Key stability and escaping

- **WHEN** a run key contains a slash and a step is retried or redriven
- **THEN** its canonical address percent-encodes that slash and the same final `/step-id` key is passed each time

#### Scenario: Reusing a deleted run address

- **WHEN** a host deletes and recreates the same run address
- **THEN** its steps receive the same external keys, and the guide warns against using that address for a different job

### Requirement: Flows have tested examples and pinned host contracts

The explicit steps.rs example SHALL remain. A sibling onboarding-flow example SHALL use the same external-services
interface and demonstrate typed creation, Continue, App hosting and recovery. Its contract SHALL be pinned in sample
service fixtures; a test-flow fixture SHALL pin the shared journal including Sleep. The guide SHALL show host contract
snapshots and ordinary state/result reads. No fixed built-in flow type name SHALL be added to the ready-made catalog.

#### Scenario: Reading a result

- **WHEN** a host calls FlowState::result<T>(id)
- **THEN** a Step result returns Some(decoded result), a bad requested T returns Some(error), and absent/Sleep returns None

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
this release SHALL execute single and joined all/first waits with or without a deadline in the running phase.

#### Scenario: Invalid outcome

- **WHEN** an open wait selects replies, a selected value is absent, selections are repeated/out of order, all/first
  cardinality is wrong, or timedOut has no deadline
- **THEN** typed decoding refuses the state

#### Scenario: Partial timeout and interrupted main wait

- **WHEN** an all wait with a deadline times out with a strict subset of stored replies in request order
- **THEN** its closed outcome is structurally valid
- **AND** a cleanup wait can record that outcome while its interrupted main wait remains open under cancelling/cancelled
