## MODIFIED Requirements

### Requirement: Closing a SQL store is resumable after caller cancellation

On first poll, SQL store close SHALL mark the store unavailable, close its change hub and stop new resource admission
before awaiting cleanup. Every caller SHALL resume or await serialized cleanup, returning after its retained steps
complete rather than merely observing the closed flag. Cancellation SHALL leave the store closed and cleanup resumable
by a later caller. No background shutdown task or fixed completion deadline SHALL be required.

#### Scenario: Cancelled then retried

- **WHEN** a caller polls close until it waits for cleanup and then drops the future
- **THEN** new operations remain unavailable and existing/new change streams end
- **AND** a later close resumes unfinished cleanup and returns after the remaining retained steps complete

#### Scenario: Concurrent callers

- **WHEN** two callers close a store whose cleanup is blocked by an existing operation
- **THEN** neither returns just because the closed flag is set
- **AND** if the cleanup owner is cancelled, a waiting caller takes over, completing after the blocker is released

#### Scenario: Never polled

- **WHEN** a close future is created and dropped without polling
- **THEN** it has no effect: the store still accepts work and its change hub stays open

#### Scenario: Concurrent first polls

- **WHEN** callers on different threads enter close at the same time
- **THEN** each completes the idempotent admission-stop prefix before its first await
- **AND** no caller skips unfinished prefix actions merely because another caller set the closed flag

#### Scenario: No retry yet

- **WHEN** all close callers are cancelled while Store handles still exist and retained cleanup is unfinished
- **THEN** the store remains closed but unfinished cleanup need not progress until another close or final resource drop
- **AND** polling or dropping the cancelled close schedules no Cicada background shutdown task; driver-owned
  connection cleanup on the store runtime is permitted and is not attributed to Cicada shutdown
- **AND** the documentation states that cancellation does not itself schedule a shutdown task
