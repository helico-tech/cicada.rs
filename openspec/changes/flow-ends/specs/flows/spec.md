## ADDED Requirements

### Requirement: A run ends with its output

`Flow` SHALL have an output type, and `run` SHALL return it. A done run SHALL keep the output in its state, checked like
a step's result; an output that fails the check SHALL be a refusal of the runner. The state SHALL hold an output only
when the run is done.

#### Scenario: A run with a result

- **WHEN** a flow's `run` returns `Ok(42)` after its last step
- **THEN** the run is done and its state holds the output `42`

#### Scenario: An output that cannot be saved

- **WHEN** a flow's `run` returns an output 62 containers deep
- **THEN** the turn fails for good, the run stays running, and a public `Continue` after a fix recovers it

### Requirement: The flow code's errors end a run as failed

The flow code's error (one `run` returns, or a step closure's error, even one the flow catches) SHALL end the main code
when it is permanent, when the message has no retry policy, or when `failures + 1` reaches its policy's attempts.
Earlier attempts SHALL fail the turn as before. The runner's refusals, failures while applying an entry, killed turns
and panics SHALL NOT end the main code.

#### Scenario: The last attempt

- **WHEN** a step fails with a transient error on every attempt of a `Continue` whose policy allows 3
- **THEN** the first two attempts fail the turn, the third marks the run cancelling with the step's error as reason, and
  after the default cleanup the run is `failed` with that reason

#### Scenario: A permanent error

- **WHEN** a step fails with a permanent error on the first attempt
- **THEN** that turn starts the cleanup, and the run ends `failed` once it succeeds

#### Scenario: A lost claim

- **WHEN** a `Continue` whose policy allows 3 attempts lost one claim and failed twice, and fails again
- **THEN** that third failure ends the main code, not the second

#### Scenario: A replay mismatch

- **WHEN** a deploy changed the flow so that replay finds another step at a saved id
- **THEN** the turn fails for good, the run stays running, and a public `Continue` after the fix recovers it

### Requirement: A failed main code runs the cleanup, then ends failed

The turn that ends the main code SHALL succeed: it SHALL keep the reason, mark the run cancelling and arm a cleanup
`Continue` with the flow's policy, as a `Cancel` does. When the cleanup succeeds, the run SHALL end `failed`, not
cancelled, with no armed `Continue`.

#### Scenario: A failed run frees its resources

- **WHEN** a flow that reserved a resource fails its main code for good, and its `on_cancel` releases the resource
- **THEN** the cleanup runs, releases it, and the run ends `failed` with the main code's reason

### Requirement: A failure reason can always be stored

The reason SHALL be the main code's error text, or `cleanup: ` and the cleanup's error text (after the main reason and
`; ` when both failed), with NUL replaced by U+FFFD and cut at 1,000 characters on a character boundary, ending in `…`
when cut. The state SHALL hold a reason only when the run failed, or is cancelling after its main code failed.

#### Scenario: A long reason with a NUL

- **WHEN** the last attempt fails with a 5,000-character error text that holds a NUL
- **THEN** the saved reason has 1,000 characters ending in `…`, and no NUL

## MODIFIED Requirements

### Requirement: A flow is hosted as an ordinary actor

Flow SHALL supply a host-chosen TYPE, a serializable/schema-described Input and Output, an async run method returning
Result<Output, HandlerError>, and retry() defaulting to the crate's shared policy. FlowActor<F> SHALL implement Actor
with FlowRun<F> as ref and FlowState<F::Input, F::Output> as state. Ordinary App hosting and explicit create-and-Continue SHALL suffice; no core hook is added.

#### Scenario: Host start

- **WHEN** a host registers FlowActor, creates a typed run with FlowState::new(input), and sends Continue
- **THEN** it starts using ordinary actor operations, also when create-and-send occur in one producer turn

### Requirement: The public step API uses ordinary futures and errors

Flow::run SHALL accept `&mut Step` and owned Input and return a Send future of Result<Output, HandlerError>. Step::run SHALL accept
an id and a Send FnOnce(String) returning a Send future of Result<T, HandlerError>, where T is Serialize,
DeserializeOwned and Send. The String SHALL be the external idempotency key. Step SHALL expose no Turn. A new send helper SHALL buffer actor effects with its journal entry through the runner.

#### Scenario: A typed external step

- **WHEN** a flow awaits a step closure that returns Result<String, HandlerError> and propagates it with `?`
- **THEN** it compiles with ordinary async code and receives a String when that result is replayed

### Requirement: Run state has a stable journal representation

FlowState SHALL contain input, journal, status, armed, replies, cleanup, output and failure. Defaults SHALL be empty
journals/lists, running, null armed and no output or failure. The contract SHALL include step/sleep/send/wait entries
and running/cancelling/done/cancelled/failed status (ADR 0044, ADR 0080). FlowMsg SHALL include Continue, Reply and Cancel. Reply.value and stored reply
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

### Requirement: The first latched outcome controls a returned flow

Step SHALL latch a new entry, the original error, or permanent validation error before returning private suspension.
Later Step calls SHALL invoke no closure. After Flow::run returns, the latch SHALL override its return. Without a latch,
an error SHALL fail normally and Ok SHALL complete only after consuming saved entries. The latch SHALL NOT preempt code.

#### Scenario: Swallowed suspension

- **WHEN** a flow catches the first new step's error, tries another step and returns Ok or another error
- **THEN** only the first closure ran and only its result and continuation commit

#### Scenario: Swallowed step failure

- **WHEN** a closure returns a transient or permanent HandlerError and flow code catches it and returns Ok
- **THEN** the original error still decides the turn: it fails the turn with its original retryability, or ends the run's
  main code as failed when it is permanent or on the last attempt, and no new progress commits

#### Scenario: Business error as data

- **WHEN** a step successfully returns a serializable Result<T, String> value containing Err
- **THEN** that business outcome is saved and can be used for branching on replay

#### Scenario: Misuse that never returns

- **WHEN** flow code swallows suspension then hangs or panics
- **THEN** ordinary core timeout/panic semantics apply and the latched new result does not commit


### Requirement: Completed and stalled runs use ordinary controls

Main completion SHALL set status done and armed null; done, cancelled and failed runs SHALL ignore Continue and remain
until deletion. Cancelling a pending continuation message, purging it, or its failing for good for any cause other than
the flow code's error SHALL leave saved progress available; another public Continue SHALL recover a running or cancelling run in
its active phase, resetting attempts by arming a fresh message. Ordinary watches, pause, resume, delete and claim-limit
semantics SHALL apply. The runner SHALL install no watch of its own.

#### Scenario: Completed run

- **WHEN** a completed run receives Continue again
- **THEN** no closure or continuation runs and its saved results remain readable

#### Scenario: Stalled continuation

- **WHEN** a running or cancelling run's armed message is cancelled, purged, or fails terminally through a refusal, a
  killed turn or a panic, then a public Continue is sent
- **THEN** a fresh armed message resumes the active phase from its saved progress using the flow's policy

#### Scenario: Out of retries

- **WHEN** an armed step exhausts its retries
- **THEN** the last attempt's turn commits the run as failed instead of failing, its message does not end failed, and
  earlier saved entries remain

#### Scenario: Failed run

- **WHEN** a run ended failed and the host sends Continue
- **THEN** no flow code runs, its status and reason stay, and a new job requires a new run key

#### Scenario: Cancelled flow differs from a cancelled message

- **WHEN** successful cleanup has committed cancelled status and the host sends Continue
- **THEN** normal work does not resume and the retained journal remains readable
- **AND** a new job requires a new run key
