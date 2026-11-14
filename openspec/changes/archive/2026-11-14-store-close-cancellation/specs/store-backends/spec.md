## ADDED Requirements

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
- **AND** the documentation states that cancellation does not itself schedule a shutdown task

### Requirement: SQL shutdown retains the backend cleanup boundaries

PostgreSQL close SHALL mark its main pool closed before awaiting cleanup, retain the listener join handle across
caller cancellation, and await borrowed main-pool connections on completion. SQLite close SHALL await its writer and
close idle readers without waiting for reads already in progress. Driver-owned cleanup MAY outlive dropped connection
futures; no synchronous socket-close or graceful runtime-teardown guarantee SHALL be inferred.

#### Scenario: PostgreSQL borrowed connection

- **WHEN** a main-pool connection is held while close is polled and cancelled
- **THEN** the pool is already closed to acquisition and a second close stays pending while the connection is held
- **AND** after the connection is released, the second close finishes, main-pool size is zero, and another close completes

#### Scenario: Cancellation while joining the listener

- **WHEN** a close caller is cancelled while joining the aborted listener task
- **THEN** the join handle remains in shared cleanup progress and a later caller joins it before marking cleanup done
- **AND** the listener's scoped probe still ends with the listener as specified by its ownership rule

#### Scenario: SQLite writer and settle work

- **WHEN** a write or its settle task holds the writer while close is polled and cancelled
- **THEN** the writer and untouched reader connections remain reachable for cleanup
- **AND** after that operation settles, a later close removes the writer and drains idle readers

#### Scenario: Cancellation during connection close

- **WHEN** a caller is cancelled after taking a connection for its own awaited close
- **THEN** the connection is dropped into the driver's existing cleanup path, while untouched connections remain for a later caller
- **AND** later close completion does not claim to join that already-dropped driver's cleanup

#### Scenario: Existing SQLite reader returns

- **WHEN** a reader is in flight during close and returns afterward, or races the idle-reader drain
- **THEN** close need not wait for that reader
- **AND** the return checks closed while holding the reader-pool lock, so the connection is dropped rather than refilling a drained pool

#### Scenario: Final drop and runtime shutdown

- **WHEN** the final Store handle is dropped after a cancelled close, or its runtime shuts down
- **THEN** retained resources follow existing Tokio/sqlx destruction behavior without a newly spawned Cicada shutdown task
- **AND** documentation does not promise awaited graceful driver cleanup after runtime shutdown

#### Scenario: In-memory store

- **WHEN** memory-store close is polled
- **THEN** it still completes synchronously, with unavailable operations and terminal change streams, and needs no resumable gate
