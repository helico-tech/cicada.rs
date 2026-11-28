## MODIFIED Requirements

### Requirement: Cleanup failures and observations follow ordinary flow recovery

Cleanup continuations SHALL use Flow::retry; handler failures SHALL keep cancelling progress until the cleanup code's
error is permanent or on the last attempt, which ends the run as failed, while unapplied/unknown commits follow core
lease and claim-limit rules. Public Continue SHALL recover the active cleanup phase; repeated
Cancel SHALL not. Delete SHALL stay immediate without finalization and pause SHALL delay cleanup. No automatic
resource cleanup or synthetic core notification SHALL be promised. Application sends and terminal status are separate turns.

#### Scenario: Exhausted or lost cleanup continuation

- **WHEN** cleanup retries exhaust through a refusal, a killed turn or a panic, or its armed message is purged
- **THEN** the run stays cancelling and public Continue normalizes then replays saved cleanup progress
- **AND** a parked cleanup wait remains parked until its reply or supported timeout, without polling

#### Scenario: Commit uncertainty

- **WHEN** phase-entry or final-status commit acknowledgement is lost
- **THEN** callers cannot assume rollback or re-run a returned handler merely on that assumption
- **AND** rereads and ordinary fenced recovery preserve whichever transaction committed

#### Scenario: Notification and deletion

- **WHEN** cleanup sends an application notification as its last helper
- **THEN** that send commits before the later turn recording cancelled, not atomically with it
- **WHEN** the run is deleted while cancelling
- **THEN** deletion runs no hook; ordinary core child/removal semantics apply
- **AND** successful finalization emits no synthetic Failed/Stopped

#### Scenario: Cleanup code out of retries

- **WHEN** a cancelling run's `on_cancel` fails on every attempt its policy allows
- **THEN** the last attempt commits status `failed` with a reason holding `cleanup: ` and the error, and later `Cancel`
  and `Continue` change nothing
