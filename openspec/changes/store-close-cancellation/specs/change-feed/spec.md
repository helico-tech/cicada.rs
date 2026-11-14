## MODIFIED Requirements

### Requirement: PostgreSQL reconnect announces readiness only after LISTEN

The listener SHALL publish typed Up/Down/Payload source events. Both first connection and reconnection SHALL announce
Up only after LISTEN succeeds. PgListener eager reconnect SHALL be disabled; try_recv Ok(None) SHALL trigger the
store's reconnect loop. Wake-up streams SHALL wake on Up and lag and retain type filtering. Store close SHALL stop
and join listener work, including its scoped probe, through resumable cleanup, and close the hub without idle feed transactions.

#### Scenario: Lost connection

- **WHEN** the listener's backend is terminated or sqlx signals connection loss with Ok(None)
- **THEN** it transitions Down, reconnects and performs LISTEN before announcing Up
- **AND** a feed resync after that Up is followed by hints from later covered writes, while ordinary workers are woken

#### Scenario: Resuming close

- **WHEN** a store close caller stops waiting for listener shutdown and another close follows
- **THEN** listener start stays disabled, the hub remains Closed and the retained join is completed by resumed cleanup
- **AND** this does not await driver-owned cleanup of the attempt's pool or reopen the feed

### Requirement: The probe cannot outlive its listener future

The PostgreSQL probe SHALL run inside the listener's future, not as a detached task. Dropping that future or ending
its connection attempt SHALL drop its probe and release Cicada's handles for that attempt. Driver-owned connection
cleanup MAY finish later; the listener SHALL NOT await that cleanup. Probe cadence, receive timeout and reconnect
behavior SHALL remain unchanged. Cancelled/concurrent Store::close completion SHALL follow the resumable store-close contract.

#### Scenario: Close with a live probe connection

- **WHEN** a healthy PostgreSQL store is closed while its listener awaits notifications and its probe has sent a notification
- **THEN** the listener task and its scoped probe are dropped by the existing shutdown
- **AND** both captured backend PIDs disappear from pg_stat_activity within the test's bounded cleanup wait
- **AND** existing and later change streams end, later store operations are unavailable, and the listener is not restarted

#### Scenario: A connection attempt ends

- **WHEN** receive ends because of a connection error, loss or silence timeout
- **THEN** its probe future is dropped before the reconnect attempt, including a pending probe query
- **AND** a new connection still completes LISTEN before announcing Up

#### Scenario: Listener task cancellation

- **WHEN** the listener task is aborted or its future is dropped while the probe is sleeping or awaiting I/O
- **THEN** dropping the listener future also drops the probe, without a separate probe task left to retain the pool
- **AND** SQL driver cleanup may retain resources temporarily without imposing a synchronous socket-close promise
