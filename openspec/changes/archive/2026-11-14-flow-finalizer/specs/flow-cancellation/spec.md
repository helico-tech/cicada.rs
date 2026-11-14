## MODIFIED Requirements

### Requirement: Cancel ends running flow work in one atomic turn

FlowMsg::Cancel in running status SHALL bypass Continue normalization, cancel the saved armed message unless it is
the current message, set cancelling and arm a cleanup Continue with Flow::retry and incoming correlation. These effects and completion of Cancel SHALL commit atomically.
The handler SHALL invoke no flow code, input clone or extra result decoding beyond ordinary state loading. It SHALL
preserve input, both journals and replies, and invoke no user cleanup until a later executing Continue.

#### Scenario: Never started or stalled

- **WHEN** Cancel reaches a new run, a parked run, a missing-timer run or a run with a permanent replay mismatch
- **THEN** it becomes cancelling without invoking either hook, with its new cleanup continuation armed
- **AND** prior saved input/progress/replies remain unchanged

#### Scenario: Armed message and current id

- **WHEN** Cancel runs with another armed message still pending
- **THEN** that message ends cancelled and no handler runs for it
- **WHEN** the armed id is absent, no longer pending or equals the current Cancel id in supplied progress
- **THEN** Cancel still completes normally without trying to cancel its own message

#### Scenario: Atomic failure

- **WHEN** the cancellation commit does not apply
- **THEN** neither phase change, cleanup continuation nor cancellation of old armed work is saved
- **AND** unapplied work after a returned handler follows ordinary lease reclamation and claim limits
- **AND** the sender's retry policy applies to handler failure, not automatically to commit uncertainty

#### Scenario: Unknown commit acknowledgement

- **WHEN** the cancellation commit acknowledgement is lost
- **THEN** callers cannot assume rollback and may reread status or send a fresh idempotent Cancel
- **AND** whichever result committed, recovery does not undo progress or restart a cancelled flow

### Requirement: Cancellation preserves terminal state and interrupted progress

Cancelled SHALL be terminal and retained until deletion. Continue SHALL do nothing and Reply SHALL be ignored before
value checks. Repeated Cancel in cancelling/cancelled and Cancel in done SHALL succeed without state writes. Open main waits and saved sleeps
SHALL remain unchanged. In cancelling, Continue SHALL normalize or execute cleanup and Reply SHALL use cleanup routing; normal flow code
SHALL NOT execute again. Cancel SHALL NOT repair undecodable state or restart a cancelled job.

#### Scenario: Interrupted waits and sleeps

- **WHEN** Cancel interrupts a sleep, parked wait or timed join with partial replies
- **THEN** its main timer is cancelled, the open wait/journal/replies stay unchanged and cleanup is armed
- **AND** later eligible replies may serve cleanup, while replies after cancelled are ignored before value checks

#### Scenario: Terminal commands

- **WHEN** done or cancelled receives Cancel again
- **THEN** it succeeds without state/version change or flow code
- **WHEN** cancelled receives Continue
- **THEN** it stays cancelled and no continuation is sent

#### Scenario: Reserved cleanup phase

- **WHEN** structurally valid cancelling state receives Continue, Reply or Cancel
- **THEN** Continue normalizes or executes cleanup, Reply applies cleanup acceptance/routing, and Cancel is a no-op

### Requirement: Cancel is mailbox-serial and does not preempt handlers

Cancel SHALL obey ordinary (deliverAt,seq) handling order and SHALL NOT interrupt a running handler. It SHALL stop
future normal replay only after its commit. A prior step or reply/timeout decision MAY already have committed; saved
effects and outcomes SHALL NOT be undone. Pauses, purge, delayed delivery and delete SHALL retain their core behavior.

#### Scenario: Running handler and retry backoff

- **WHEN** Cancel queues while a handler is running
- **THEN** that handler may finish first and Cancel then cancels its successor
- **WHEN** Cancel is due before an armed retry's backoff ends
- **THEN** it can cancel that retry without rerunning the step
- **AND** previously attempted external effects are not compensated

#### Scenario: Reply or timer decision race

- **WHEN** a reply/deadline decision commits before Cancel but its next Continue has not run
- **THEN** Cancel preserves the closed decision and prevents later normal flow code from consuming it; cleanup may inspect the stable saved result
- **WHEN** Cancel commits first
- **THEN** the main timer is cancelled and later eligible replies route to cleanup without closing the interrupted main wait
- **AND** equal deliverAt races follow seq in either order

#### Scenario: Finishing and queued continuations

- **WHEN** Cancel runs after the last helper commits but before done is recorded
- **THEN** the status becomes cancelling despite completed prior effects, and cleanup runs before cancelled
- **WHEN** done was already recorded
- **THEN** Cancel is a no-op
- **AND** queued public Continues during cancelling normalize cleanup; after cancelled they are no-ops

#### Scenario: Pause purge and delete

- **WHEN** the run is paused
- **THEN** Cancel waits for resume
- **WHEN** a queued Cancel is purged or the run deleted
- **THEN** no cancellation handler runs
- **AND** sending Cancel to an absent run retains ordinary send/commit rejection, while create-and-send can commit together

### Requirement: Cancellation uses ordinary observation and trust boundaries

A successful Cancel SHALL be an ordinary done turn, preserving incoming correlation on its cleanup Continue. It SHALL
NOT emit synthetic Failed/Stopped, stop/delete the run, purge its mailbox or control targets/children. Its state write
MAY produce an ordinary best-effort change hint. Cancel SHALL have no sender check beyond existing host authorization;
receipt/enqueue acknowledgement SHALL NOT mean cancellation completed.

#### Scenario: Observers and watches

- **WHEN** running enters cancelling or cleanup records cancelled successfully
- **THEN** observers see the corresponding Cancel or cleanup Continue complete, the actor remains readable and watches receive no synthetic notification
- **AND** the cancelled armed message has no handler turn or watch notification
- **AND** clients can reread status after a change hint without relying on hints as guaranteed completion events

#### Scenario: Targets and senders

- **WHEN** an authorized outside sender or another actor sends Cancel
- **THEN** the same transition applies without matching from to any target or owner
- **AND** no automatic target/child control occurs; application cleanup may act explicitly, and eligible replies serve cleanup until terminal status

### Requirement: Cancellation rollout distinguishes flow and core controls

Docs SHALL distinguish FlowMsg::Cancel, pending-message cancellation, operator running-turn cancellation, pause,
purge, delete and Continue. All hosts/workers SHALL be upgraded before enabling Cancel despite equal hashes. A refusal
by an older host SHALL NOT imply the run stopped. Runs cancelled before finalizers are implemented SHALL NOT be
retroactively finalized, and this piece SHALL run only application-defined cleanup, without inferring resources or automatic compensation.

#### Scenario: Refused Cancel recovery

- **WHEN** an older host refuses Cancel and the run stays running
- **THEN** after upgrading every host/worker, a fresh Cancel, operator retry or dead-letter redrive can cancel safely
- **AND** the new sender/id is acceptable because Cancel is idempotent and has no Reply-style sender check

#### Scenario: Core controls and later cleanup

- **WHEN** one continuation is cancelled or an operator requests running-turn cancellation
- **THEN** ordinary core behavior may stall a running flow but does not itself set FlowStatus::Cancelled
- **WHEN** a run was already cancelled by a release without finalizer support before this release is installed
- **THEN** that terminal run is not automatically reopened or finalized

#### Scenario: Contract compatibility

- **WHEN** snapshots are regenerated
- **THEN** existing flow hashes and description-stripped schemas equal the landed cancellation predecessor
- **AND** any flow snapshot differences are descriptions only, with other ready-made actor contracts unchanged

#### Scenario: Mixed finalizer implementations

- **WHEN** a predecessor or host with the default hook handles cancellation for a type intended to have cleanup
- **THEN** it can record cancelled without that cleanup despite an equal contract hash
- **AND** docs require every host/worker to run the intended hook before relying on it, or a new flow type name
