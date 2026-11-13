## ADDED Requirements

### Requirement: A new sleep commits one fixed wake time

Step::sleep(id, duration) SHALL be async and lazy, returning HandlerResult. On first execution it SHALL save a Sleep
entry with until equal to store claim time plus duration, rounded up to microseconds, and suspend. The runner SHALL
commit the entry and one armed self-Continue due at until together. No handler SHALL remain alive to wait for that time.
Zero duration SHALL still end the turn through this commit boundary.

#### Scenario: First durable wait

- **WHEN** a flow reaches a new 60-second sleep at store time zero
- **THEN** it commits one Sleep entry with until 60 seconds and one armed continuation due then
- **AND** no following step closure runs in that turn

#### Scenario: Zero sleep

- **WHEN** a flow reaches a new zero-duration sleep
- **THEN** it commits the Sleep and continuation before another turn can advance past it

#### Scenario: Fractional microsecond

- **WHEN** a new sleep lasts one nanosecond from a whole-microsecond claim time
- **THEN** until is the next microsecond, never earlier

### Requirement: Invalid new sleeps fail permanently

A new sleep SHALL use the runner's id uniqueness/order checks and 1000-entry limit. Duration SHALL be at most
MAX_DELAY, with zero allowed. Addition, microsecond rounding and timestamp conversion SHALL be checked, and until
SHALL NOT exceed MAX_INSTANT. A violation SHALL latch permanent failure with no new entry or continuation committed.

#### Scenario: Sleep bounds

- **WHEN** a duration exceeds MAX_DELAY or its rounded wake time exceeds MAX_INSTANT
- **THEN** the turn fails permanently with earlier progress and armed unchanged

#### Scenario: Shared id namespace and count

- **WHEN** sleep repeats a step/sleep id in one execution or would append entry 1001
- **THEN** it fails permanently and does not arm a wait

### Requirement: Replay honors the committed deadline

A replayed sleep SHALL match the cursor's id and Sleep kind. Before until it SHALL latch Waiting and rearm at that same
until without adding an entry. At/after until it SHALL advance and return Ok without a new timer. The newly supplied
duration SHALL be ignored on replay; the stored time is authoritative. A kind/id mismatch SHALL fail permanently.

#### Scenario: Before and at the deadline

- **WHEN** replay reaches a sleep before its saved until
- **THEN** the journal is unchanged and a continuation is armed for that until
- **WHEN** replay reaches it at until
- **THEN** it returns and the next new operation may run in that turn

#### Scenario: Duration changes in code

- **WHEN** a saved sleep is replayed with a different duration, including one that would be invalid for a new sleep
- **THEN** the saved until still determines waking and is not recomputed

#### Scenario: Wrong operation kind

- **WHEN** sleep encounters a saved Step at its cursor, or run encounters a saved Sleep
- **THEN** the runner fails permanently rather than skipping or reinterpreting the entry

### Requirement: Recovery and early delivery cannot skip sleep

A public Continue, retry/redrive or deliver-now SHALL retain the runner's armed-message protocol and replay checks.
If the saved until has not passed, execution SHALL rearm for it without running subsequent closures. The previous
armed id SHALL be cancelled only when it differs from the current message. Self-continuations SHALL use F::retry().

#### Scenario: Manual early Continue

- **WHEN** an operator sends Continue while a run is sleeping
- **THEN** its normalization turn arms a current continuation, which replays and rearms at the original until
- **AND** only one continuation is armed and no subsequent closure runs early

#### Scenario: Deliver-now

- **WHEN** the armed sleep continuation is made due early
- **THEN** it rearms at the original until without attempting to cancel its own message or skipping the sleep

#### Scenario: Early redrive

- **WHEN** a failed continuation is retried or redriven under a new id before until
- **THEN** it follows public recovery and retains the same wake time

### Requirement: Sleep uses the runner's latched control flow

Sleep SHALL inspect an existing latch before doing work. It SHALL perform its own checking/latching without an internal
await point. An unpolled sleep SHALL do nothing. New/Waiting SHALL control the turn after Flow::run returns even if it
caught suspension. A preceding abandoned run step SHALL remain a permanent failure. Whole-handler failures SHALL
retain the core's no-progress semantics.

#### Scenario: Sleep after an abandoned step

- **WHEN** flow code drops a running step future and calls sleep
- **THEN** sleep adds nothing and the returned flow still fails permanently

#### Scenario: Swallowed sleep suspension

- **WHEN** flow code catches sleep's suspension and calls another step before returning
- **THEN** that closure is not invoked and the Sleep/Waiting outcome still controls the continuation

### Requirement: Timer loss and lateness use normal actor controls

Cancel, purge or terminal failure of a continuation SHALL leave the run's saved sleep intact and stalled until a
public Continue recovers it. Pause SHALL delay handling without changing until. Delete SHALL remove the run and its
pending continuations. A wake after until SHALL proceed without starting the duration again. A later step's retries
SHALL replay the elapsed sleep without another wait.

#### Scenario: Recovery after a cancelled timer

- **WHEN** the timer is cancelled and no Continue is sent
- **THEN** the run remains stalled even after until
- **WHEN** a public Continue is sent
- **THEN** it resumes against the saved until

#### Scenario: Pause past wake time

- **WHEN** a run is paused across until and later resumes
- **THEN** it advances when claimed rather than sleeping the duration again

#### Scenario: Retry after sleep

- **WHEN** the first step after a sleep fails transiently
- **THEN** its retry replays the sleep as elapsed and retries that step without repeating earlier external closures

#### Scenario: Deleted run

- **WHEN** a sleeping run is deleted
- **THEN** its pending continuation is purged and no sleep timer recreates the run

### Requirement: Sleep preserves the established flow contract

The change SHALL add no state field, message variant, journal variant or schema difference to the runner's initial
contract. Existing test/example/sample-service flow fixtures SHALL remain unchanged. The guide SHALL show the sleep
helper, fixed-deadline recovery, limits and zero-duration boundary, and require compatible flow code on all hosting
nodes before starting runs that use it.

#### Scenario: Contract stability

- **WHEN** a flow begins using Step::sleep without changing its Input or TYPE
- **THEN** its contract hash and pinned contract text remain the same as before this change
