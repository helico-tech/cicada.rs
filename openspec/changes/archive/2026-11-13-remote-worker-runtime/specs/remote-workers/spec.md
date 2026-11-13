## ADDED Requirements

### Requirement: A Worker runs existing Rust actors through a TurnSource

The core SHALL expose Worker and an object-safe async TurnSource using core registration, lease, renewal, release,
completion and type-list results. Worker SHALL host ordinary Actor implementations and observers without a database
handle. ActorSystem and Arc<TurnSource> adapters SHALL be provided without an HTTP dependency or new feature flag.

#### Scenario: The in-process reference source

- **WHEN** a Worker hosts an existing Rust actor and uses a handler-free ActorSystem as its source
- **THEN** it registers the contract, leases messages, runs the unchanged handler and settles its state/effects
- **AND** the source needs no copy of the handler implementation

#### Scenario: Local and Worker parity

- **WHEN** equivalent actors execute locally and through a Worker using the reference source
- **THEN** their state, effects and outcomes follow the same handler, retry, timeout, decode, panic and refusal rules
- **AND** raw sends/creates, typed post-handler checks, watches, controls and observer header selection remain available

#### Scenario: Errors from the reference source

- **WHEN** the reference source reports invalid input or StoreError::Invalid, an unavailable/corrupt store, or StartError
- **THEN** its adapter maps these to SourceError::Invalid, Unavailable, or Fatal respectively
- **AND** no source error authorizes release after the handler ended

#### Scenario: Source contract

- **WHEN** a host implements TurnSource
- **THEN** it must preserve the core operations' fencing, unique claims, nonshortening renewal and explicit rejection
  rules and must not hide mutation retries
- **AND** no promise is made that Worker can make a dishonest source obey store exclusivity

### Requirement: Worker initialization checks contracts without starting a store node

Worker SHALL validate configuration, hosted definitions, duplicate types and the 100-type limit, then register exact
generated contracts. It SHALL require matching stored hashes and retain the successful host/hash map. Failed
initialization SHALL be retryable. It SHALL create no node heartbeat, sweep, listener or source lifecycle ownership.

#### Scenario: Conflicts and bad definitions

- **WHEN** a definition is invalid or duplicated, a config field is invalid, or more than 100 types are hosted
- **THEN** initialize/start fails with StartError and no handler runs
- **WHEN** registration reports a conflicting or unexpected stored hash
- **THEN** the failure includes the existing ContractConflict fields

#### Scenario: Empty Worker

- **WHEN** a Worker with no hosted actors starts
- **THEN** it starts without sending an empty lease request and remains stoppable

#### Scenario: Initialization fails then recovers

- **WHEN** registration fails and the host later retries initialize before start/stop
- **THEN** it may succeed without a new Worker, and only a successful initialization is cached

#### Scenario: Stop during registration

- **WHEN** start is waiting for a source registration and stop occurs
- **THEN** stop can mark the New Worker stopped without waiting on a lifecycle lock held across that request
- **AND** a late registration response cannot cause start to dispatch work after stop

### Requirement: Worker polling and manual batches share bounded capacity

Worker SHALL reserve concurrency slots before a lease call and hold them through handoff/execution. Background and
manual polling SHALL share that capacity. A call SHALL request at most free capacity, its requested limit and 100.
No slot SHALL become free merely because a manual caller dropped its future while cleanup is still running.

#### Scenario: Mixed manual and background work

- **WHEN** concurrent manual batches and the background loop ask for work on one Worker
- **THEN** in-flight lease reservations plus occupied turn slots never exceed configured concurrency
- **AND** unused reservations are returned and running_turns counts turn tasks rather than outstanding requests

#### Scenario: A manual batch

- **WHEN** run_due_turns is called in New or Started
- **THEN** it makes at most one bounded lease request and returns the number of turn tasks started after they settle
- **AND** limit zero or no free capacity returns zero without a lease request
- **WHEN** it is called in Stopping or Stopped
- **THEN** it returns a state error without leasing

#### Scenario: Polling cadence

- **WHEN** a batch fills the request
- **THEN** the background loop may poll again immediately for free capacity
- **WHEN** a batch is sparse/empty or the source is unavailable
- **THEN** it waits poll_interval or a completion wake, without a source due-time query or push subscription

#### Scenario: A permanent leasing failure

- **WHEN** either lease path receives ContractMismatch, SourceError::Invalid or Fatal
- **THEN** the Worker records the first automatic-stop error and starts graceful shutdown without joining itself
- **AND** stopped returns that error to every waiter; a normal explicit stop returns None

#### Scenario: A temporary manual leasing failure

- **WHEN** a manual lease call returns Unavailable
- **THEN** run_due_turns returns the error rather than hiding a lease retry, and the Worker remains usable

### Requirement: Worker execution follows conservative per-turn deadlines

Worker SHALL use each granted TTL and its own monotonic request start to compute a deadline with min(TTL/10, 1s)
margin. Renewals SHALL keep the maximum confirmed deadline. Source heartbeat intervals SHALL be max(TTL/3, 1ns),
without a configurable override. The watchdog SHALL remain independent of stalled renewal calls.

#### Scenario: Different peer TTLs

- **WHEN** renewal returns a shorter TTL and the store retains the prior expiry
- **THEN** the Worker keeps its later confirmed deadline, uses the latest TTL for subsequent heartbeats, and never
  derives a deadline from a remote wall clock

#### Scenario: Very short but valid TTL

- **WHEN** a valid grant has a TTL below one millisecond
- **THEN** no one-millisecond floor delays its heartbeat beyond the safe interval
- **AND** grants below two nanoseconds or with unrepresentable local deadlines are refused as invalid source metadata

#### Scenario: Late grant and immediate renewal

- **WHEN** a grant arrives at or after its conservative deadline
- **THEN** it receives a best-effort release and starts no observer, load or handler
- **WHEN** a valid grant arrives with less than a heartbeat remaining
- **THEN** renewal begins immediately rather than waiting another heartbeat

#### Scenario: Hanging renewal or keeper panic

- **WHEN** renewal hangs, returns errors until the deadline, reports LeaseLost, or the keeper panics
- **THEN** the watchdog/keeper marks the lease lost and aborts work under the existing cooperative cancellation rules
- **AND** no completion is started after loss

#### Scenario: Handler time

- **WHEN** a handler calls now or current_time
- **THEN** now is the claimed_at instant and current_time adds elapsed worker monotonic time since the lease request
- **AND** current_time is an estimate that may lead store time by transit and server processing time

#### Scenario: Renewal and watchdog become ready together

- **WHEN** a renewal reply and the watchdog are both ready at or after the old confirmed local deadline
- **THEN** accepting an extension checks expiration/loss atomically and cannot revive the expired grant
- **AND** polling renewal first does not permit more handler work or completion

#### Scenario: Completion starts at expiry

- **WHEN** handler work finishes at or after the confirmed local deadline, before the watchdog task has run
- **THEN** the Worker checks expiration before starting completion and sends no completion request
- **AND** a simultaneously ready renewal cannot revive that expired grant

#### Scenario: Malformed batch metadata

- **WHEN** a source returns more leases than requested, an unrequested type, duplicate actors within the batch,
  an actor/message target mismatch or invalid timing
- **THEN** the Worker starts no handler for that batch, performs guarded best-effort cleanup and treats it as a
  Fatal leasing error

### Requirement: Worker completion retries retain the current report

Worker SHALL keep a single current report and one completion budget captured from the latest confirmed TTL when
completion starts. Rejected SHALL permanently replace the report with the supplied nonretryable failure. Neither
that transition nor renewal SHALL reset the budget. Pending completion calls SHALL be bounded by budget and lease loss.

#### Scenario: Returned report rejected

- **WHEN** completion returns Rejected for a Returned report
- **THEN** the Worker discards original effects and retries only Failed with the returned cause/reason
- **AND** subsequent transport uncertainty or a newly valid original target cannot restore those effects

#### Scenario: A retryable failure's reschedule is refused

- **WHEN** the current report is Failed with retryable=true and completion returns Rejected
- **THEN** the Worker switches to the supplied nonretryable Failed report rather than treating every failed report
  as already terminal
- **AND** a Rejected result for an already nonretryable Failed report is a protocol error that abandons without release

#### Scenario: Completion retry budget

- **WHEN** completion reports Unavailable
- **THEN** the Worker retries the identical current report using backoff from 50ms up to 2s, bounded by its fixed
  completion budget and lease
- **AND** source Invalid/Fatal, exhausted budget, or LeaseLost ends attempts without release or handler re-execution

#### Scenario: A completion request never returns

- **WHEN** source.complete_turn stays pending beyond the budget or the lease is lost
- **THEN** the Worker drops the request, treats it as uncertain, ends its turn/keeper and never releases it
- **AND** a possible commit after cancellation cannot lead the Worker to submit a different report

#### Scenario: The actual completed outcome

- **WHEN** a Returned report receives Completed with a failed outcome derived by server validation
- **THEN** the Worker ends its observer using that actual failed outcome and actual sent envelopes
- **AND** only an acknowledged Completed ends an observer; uncertain or lost turns drop it

#### Scenario: An acknowledgement is lost

- **WHEN** completion applies but its answer is lost, including when retention hides the outcome
- **THEN** a retry cannot duplicate store effects and LeaseLost is not treated as proof of success
- **AND** the Worker performs no GET-based reconciliation that claims to resolve every uncertain outcome

### Requirement: Worker shutdown preserves handler-end and cleanup boundaries

Worker SHALL stop polling, drain its tracked turns for the grace period, cancel them and wait the cancel timeout.
It SHALL release only after work has stopped and before handler end. Manual cohorts SHALL remain tracked after their
caller disappears. The Worker SHALL not stop or close its source; failed cleanup MAY leave leases to expire.

#### Scenario: Lease response after caller disappearance

- **WHEN** a detached lease call returns after its waiting receiver was dropped or shutdown won handoff
- **THEN** known leases receive best-effort release and unused capacity is returned
- **AND** a response never received may contain unknown leases that expire and count lost when replaced

#### Scenario: Drop one manual batch

- **WHEN** a caller drops run_due_turns while it has running turns
- **THEN** only that cohort is cancelled, other cohorts continue, and reservations/slots stay owned until cleanup

#### Scenario: Stop an unstarted Worker with a manual batch

- **WHEN** a New Worker has manual tasks and is stopped
- **THEN** it drains/cancels those tasks rather than treating New as having no running work

#### Scenario: Handler-end arbitration

- **WHEN** stop cancels work before the handler ended and it has actually stopped
- **THEN** the lease may be released
- **WHEN** the handler already ended
- **THEN** the Worker never releases and may finish only within its remaining completion budget or abandon
- **AND** an operator cancellation received through renewal that won against handler end still produces Cancelled;
  operator cancellation observed later is ignored

#### Scenario: Operator cancellation followed by shutdown

- **WHEN** operator cancellation received through renewal wins against handler end, then shutdown cancels the task
- **THEN** the Worker retains the Cancelled completion path rather than releasing the lease as unfinished work
- **AND** shutdown or manual-future cancellation alone releases only stopped, unfinished work

#### Scenario: Non-yielding work and source lifetime

- **WHEN** work ignores cooperative cancellation beyond the cancel timeout
- **THEN** stop can finish while that work remains, preserving existing warnings and no unsafe release
- **AND** the host keeps the source available for cleanup; Worker never invokes source stop or Store::close
