# flow-cancellation Specification

## Purpose
A public `Cancel` ends a running flow in its own turn: it cancels the armed continuation and marks the run cancelled,
while the input, journal, replies and open wait stay saved and readable. It never interrupts a running turn, undoes no
effects and runs no cleanup; a cancelled run ignores every later message.

## Requirements

### Requirement: Cancel ends running flow work in one atomic turn

FlowMsg::Cancel in running status SHALL bypass Continue normalization, cancel the saved armed message unless it is
the current message, set cancelled and clear armed. These effects and completion of Cancel SHALL commit atomically.
The handler SHALL invoke no flow code, input clone or extra result decoding beyond ordinary state loading. It SHALL
preserve input, both journals and replies, and send no continuation or automatic cleanup work.

#### Scenario: Never started or stalled

- **WHEN** Cancel reaches a new run, a parked run, a missing-timer run or a run with a permanent replay mismatch
- **THEN** it becomes cancelled without invoking Flow::run and with armed null
- **AND** prior saved input/progress/replies remain unchanged

#### Scenario: Armed message and current id

- **WHEN** Cancel runs with another armed message still pending
- **THEN** that message ends cancelled and no handler runs for it
- **WHEN** the armed id is absent, no longer pending or equals the current Cancel id in supplied progress
- **THEN** Cancel still completes normally without trying to cancel its own message

#### Scenario: Atomic failure

- **WHEN** the cancellation commit does not apply
- **THEN** neither terminal status nor cancellation of armed work is saved
- **AND** unapplied work after a returned handler follows ordinary lease reclamation and claim limits
- **AND** the sender's retry policy applies to handler failure, not automatically to commit uncertainty

#### Scenario: Unknown commit acknowledgement

- **WHEN** the cancellation commit acknowledgement is lost
- **THEN** callers cannot assume rollback and may reread status or send a fresh idempotent Cancel
- **AND** whichever result committed, recovery does not undo progress or restart a cancelled flow

### Requirement: Cancellation preserves terminal state and interrupted progress

Cancelled SHALL be terminal and retained until deletion. Continue SHALL do nothing and Reply SHALL be ignored before
value checks. Repeated Cancel and Cancel in done SHALL succeed without state writes. Open main waits and saved sleeps
SHALL remain unchanged. In this release, every message in cancelling SHALL fail permanently with a finalizer-release
diagnostic. Cancel SHALL NOT repair undecodable state or restart a cancelled job.

#### Scenario: Interrupted waits and sleeps

- **WHEN** Cancel interrupts a sleep, parked wait or timed join with partial replies
- **THEN** its timer is cancelled, the open wait/journal/replies stay unchanged and armed is null
- **AND** later replies, including values too deep for storage, are ignored without state/version changes

#### Scenario: Terminal commands

- **WHEN** done or cancelled receives Cancel again
- **THEN** it succeeds without state/version change or flow code
- **WHEN** cancelled receives Continue
- **THEN** it stays cancelled and no continuation is sent

#### Scenario: Reserved cleanup phase

- **WHEN** structurally valid cancelling state receives Continue, Reply or Cancel
- **THEN** the message fails permanently without effects because this release does not implement finalization

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
- **THEN** Cancel preserves the closed decision and prevents later flow code from consuming it
- **WHEN** Cancel commits first
- **THEN** the armed timer is cancelled and later replies are ignored
- **AND** equal deliverAt races follow seq in either order

#### Scenario: Finishing and queued continuations

- **WHEN** Cancel runs after the last helper commits but before done is recorded
- **THEN** the status becomes cancelled despite completed prior effects
- **WHEN** done was already recorded
- **THEN** Cancel is a no-op
- **AND** queued public Continues after cancellation are no-ops while earlier ones retain ordinary normalization

#### Scenario: Pause purge and delete

- **WHEN** the run is paused
- **THEN** Cancel waits for resume
- **WHEN** a queued Cancel is purged or the run deleted
- **THEN** no cancellation handler runs
- **AND** sending Cancel to an absent run retains ordinary send/commit rejection, while create-and-send can commit together

### Requirement: Cancellation uses ordinary observation and trust boundaries

A successful Cancel SHALL be an ordinary done turn, preserving incoming correlation and sending no message. It SHALL
NOT emit synthetic Failed/Stopped, stop/delete the run, purge its mailbox or control targets/children. Its state write
MAY produce an ordinary best-effort change hint. Cancel SHALL have no sender check beyond existing host authorization;
receipt/enqueue acknowledgement SHALL NOT mean cancellation completed.

#### Scenario: Observers and watches

- **WHEN** running becomes cancelled successfully
- **THEN** observers see the Cancel turn complete, the actor remains readable and watches receive no synthetic notification
- **AND** the cancelled armed message has no handler turn or watch notification
- **AND** clients can reread status after a change hint without relying on hints as guaranteed completion events

#### Scenario: Targets and senders

- **WHEN** an authorized outside sender or another actor sends Cancel
- **THEN** the same transition applies without matching from to any target or owner
- **AND** previously requested jobs and child actors remain running/existing, with later replies ignored

### Requirement: Cancellation rollout distinguishes flow and core controls

Docs SHALL distinguish FlowMsg::Cancel, pending-message cancellation, operator running-turn cancellation, pause,
purge, delete and Continue. All hosts/workers SHALL be upgraded before enabling Cancel despite equal hashes. A refusal
by an older host SHALL NOT imply the run stopped. Runs cancelled before finalizers are implemented SHALL NOT be
retroactively finalized, and this piece SHALL promise no resource cleanup.

#### Scenario: Refused Cancel recovery

- **WHEN** an older host refuses Cancel and the run stays running
- **THEN** after upgrading every host/worker, a fresh Cancel, operator retry or dead-letter redrive can cancel safely
- **AND** the new sender/id is acceptable because Cancel is idempotent and has no Reply-style sender check

#### Scenario: Core controls and later cleanup

- **WHEN** one continuation is cancelled or an operator requests running-turn cancellation
- **THEN** ordinary core behavior may stall a running flow but does not itself set FlowStatus::Cancelled
- **WHEN** a run was cancelled under this release and finalizer support is later installed
- **THEN** that terminal run is not automatically reopened or finalized

#### Scenario: Contract compatibility

- **WHEN** snapshots are regenerated
- **THEN** existing flow hashes and description-stripped schemas equal the landed join predecessor
- **AND** any flow snapshot differences are descriptions only, with other ready-made actor contracts unchanged
