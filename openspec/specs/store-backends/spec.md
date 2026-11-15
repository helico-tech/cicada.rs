# store-backends Specification

## Purpose
Which store backends exist, how a host opens one and hands it to a node, and what of a store a host can and cannot
reach. A store is not an extension point: the backends are the ones in this repository.

## Requirements

### Requirement: A host opens a store through the core

The core SHALL offer one public type, `Store`. A host SHALL open one with `Store::sqlite(path, options)` or
`Store::postgres(url, options)` and pass it to `ActorSystem::builder`. A `Store` SHALL be cheap to clone, and its
clones SHALL share one backend. The host SHALL close the store it opened with `close`; a system SHALL NOT hand the
store back.

#### Scenario: A node on SQLite

- **WHEN** a host opens a store with `Store::sqlite` on a new path, builds a system on it, creates an actor and sends
  it a message
- **THEN** the message is handled and the actor's state is in the file

#### Scenario: A node on PostgreSQL

- **WHEN** a database URL is set for the tests, and a host opens a store with `Store::postgres`, builds a system on
  it, creates an actor and sends it a message
- **THEN** the message is handled

#### Scenario: Two systems on one store

- **WHEN** a host clones a `Store` and builds two systems on the clones, one that hosts a type and one that hosts
  nothing
- **THEN** a message sent through the second system is handled by the first

#### Scenario: Closing and opening again

- **WHEN** a host stops its system, closes its SQLite store and opens the same file again
- **THEN** the open succeeds and the data is there

### Requirement: A closed store answers unavailable

After `close`, store operations SHALL fail as unavailable on every backend, except change subscriptions, which
SHALL end normally with no items. Existing change streams SHALL also end, discarding backlog. Nonstreaming HTTP
requests that need the store SHALL get `503 store-unavailable`; GET /changes SHALL instead return an empty ended
200 SSE body, and existing SSE bodies SHALL end without a terminal event. Worker stop SHALL NOT close the store.

#### Scenario: A request after the store is closed

- **WHEN** a host closes its store and a client then sends a message over HTTP to a system on that store
- **THEN** the answer is `503 store-unavailable`

#### Scenario: Health after the store is closed

- **WHEN** a host closes its store and a client then gets `/health`
- **THEN** the answer is `503`

#### Scenario: Every backend

- **WHEN** a store of each backend is closed and then asked for its time
- **THEN** each fails as unavailable

#### Scenario: A change subscription after close

- **WHEN** a system calls changes after its Store was closed, or a previous stream is polled after close
- **THEN** the stream ends with None instead of yielding a store error or Resync

#### Scenario: HTTP changes after close

- **WHEN** GET /changes is requested after store close
- **THEN** the answer is 200 text/event-stream with an empty ended body, not a 503 problem
- **AND** an already open change stream ends without a final SSE event

### Requirement: Each SQL backend is a cargo feature

`Store::sqlite` SHALL exist only with the feature `sqlite`, and `Store::postgres` only with the feature `postgres`.
No feature SHALL be on by default. With one of the two features the core SHALL build without the driver of the
other. With no feature the core SHALL build.

#### Scenario: Only SQLite

- **WHEN** the core is built with the feature `sqlite` alone
- **THEN** the build passes, and `cargo tree` for that build lists no PostgreSQL driver

#### Scenario: Only PostgreSQL

- **WHEN** the core is built with the feature `postgres` alone
- **THEN** the build passes, and `cargo tree` for that build lists no SQLite driver

#### Scenario: No feature

- **WHEN** the core is built with no feature
- **THEN** the build passes

### Requirement: A store is not an extension point

Code outside the core SHALL NOT be able to implement a store backend or to call one. The public API SHALL NOT name
the backend trait or the types that only a backend needs, such as the request of a commit and a claim. The types a
host reads SHALL stay public: messages, outcomes, pages, ids, and the views of nodes and running turns. No type of
the SQL driver SHALL be part of the public API.

#### Scenario: A host names a backend type

- **WHEN** a doc test marked `compile_fail` names the request of a commit, a claim, or the backend trait from
  outside the core
- **THEN** it fails to compile, so the doc test passes

#### Scenario: A host reads an outcome

- **WHEN** a host lists outcomes through `ActorSystem`
- **THEN** it gets them as a public type, without naming a backend

### Requirement: A host reads the store's time and the next due time through the system

`ActorSystem` SHALL answer the store's time with `now`. With `next_due_at` it SHALL answer the earliest store time
at which an instance of a type the system hosts may become claimable: the delivery time of the head of its mailbox,
or the end of its lease when that is later. Paused instances SHALL NOT count, and the answer SHALL be nothing when
no message is pending.

#### Scenario: The store's time

- **WHEN** a test on virtual time moves the clock forward by one hour and asks the system for `now`
- **THEN** the answer is one hour later than before

#### Scenario: A message that is due later

- **WHEN** a message for a hosted type is sent with a delay of ten minutes and nothing else is pending
- **THEN** `next_due_at` answers the time ten minutes from now

#### Scenario: Nothing pending

- **WHEN** no message is pending for a hosted type
- **THEN** `next_due_at` answers nothing

#### Scenario: A paused actor

- **WHEN** the only pending message belongs to a paused actor
- **THEN** `next_due_at` answers nothing

### Requirement: Test support is one feature of the core

The in-memory store, the test clocks, `TestActors` and the helper for a node that dies SHALL exist only with the
feature `testing`. `cicada-test` SHALL turn it on and export `TestActors` and the clocks under the names they have
today. The in-memory store SHALL be opened with `Store::memory`, `Store::memory_on` or `Store::memory_with`. A
crate's tests on `TestActors` SHALL need no backend feature.

#### Scenario: A test on the harness

- **WHEN** a crate depends on `cicada` without a feature and on `cicada-test` for its tests, and a test runs an actor
  on `TestActors` with the imports it had before this change
- **THEN** the test builds and passes

#### Scenario: A node that claims a turn and dies

- **WHEN** a test claims a due message with the helper, as a node that commits nothing, and the lease runs out
- **THEN** a running system claims the message again, as its second attempt, and handles it

### Requirement: Every backend passes the same store tests

The conformance cases SHALL run in `cargo test --workspace` against the in-memory store and SQLite, and against
PostgreSQL when a database URL is set. Moving tests into the core SHALL NOT drop one: not of the conformance suite,
the randomised tests, the tests of the two store crates, or the tests that move out of `cicada-test`.

#### Scenario: A normal test run

- **WHEN** `cargo test --workspace` runs without a database URL
- **THEN** the conformance cases run and pass for the in-memory store and for SQLite

#### Scenario: A test run with PostgreSQL

- **WHEN** `cargo test --workspace` runs with `CICADA_PG_URL` set
- **THEN** the conformance cases run and pass for PostgreSQL too

#### Scenario: No test is lost

- **WHEN** the tests of each moved suite are counted before and after this change
- **THEN** no suite has fewer than before

#### Scenario: The tests of the core alone

- **WHEN** `cargo test -p cicada` runs with no feature named
- **THEN** the store tests run: the conformance cases for the in-memory store and for SQLite are among the tests
  that ran

### Requirement: A store on PostgreSQL can be opened over TLS

With the feature `postgres`, the core SHALL be built with TLS for PostgreSQL, with nothing else to switch on. A URL
that asks for TLS SHALL NOT fail because the build lacks TLS. The feature `sqlite` alone SHALL NOT build a TLS
library for the store.

#### Scenario: TLS is asked from a server that offers none

- **WHEN** a store is opened with `sslmode=require` on a PostgreSQL server that offers no TLS
- **THEN** the open fails, and the error says that the server does not support TLS, not that the build lacks it

#### Scenario: A URL without a mode

- **WHEN** a store is opened on the same server with a URL that names no `sslmode`
- **THEN** the open succeeds, as before

#### Scenario: A SQLite host

- **WHEN** the core is built with the feature `sqlite` alone
- **THEN** `cargo tree` for that build lists no TLS library

### Requirement: A store that is open keeps working when a column is added

A store that is open and has run its statements SHALL keep working when a column with a default is added to a table
of its schema, on both SQL backends, so that a later migration that only adds such a column can be additive: nodes
of the release before keep running while it is applied.

#### Scenario: A column is added under an open store on PostgreSQL

- **WHEN** a store on PostgreSQL has created an actor, sent, claimed, committed and listed, a column with a default
  is then added to each of its tables on another connection, and the store does the same operations again
- **THEN** every operation succeeds as before

#### Scenario: A column is added under an open store on SQLite

- **WHEN** a store on SQLite has run the same operations, a column with a default is added to each of its tables, and
  it runs them again
- **THEN** every operation succeeds as before

### Requirement: Renewing a held lease never shortens its expiry

For a held current epoch, every backend's renew SHALL atomically set expiry to the later of its existing expiry and
store now plus the requested TTL, and return that actual expiry. Both ordinary and cancel-requested renewals SHALL
obey this rule. Existing epoch/held-lease fencing and store-time precision SHALL remain unchanged.

#### Scenario: A shorter extension request

- **WHEN** a lease expires at t=60 and renewal at t=10 requests 5 seconds
- **THEN** its expiry remains t=60 and the renewal result returns t=60
- **AND** another claim before t=60 is excluded even if the renewal answer never reaches the holder

#### Scenario: A longer extension request

- **WHEN** a lease expires at t=60 and renewal at t=10 requests 80 seconds
- **THEN** its expiry becomes t=90 and the renewal result returns t=90

#### Scenario: Concurrent shorter and longer renewals

- **WHEN** renewals for one held epoch race with different requested intervals
- **THEN** each serializes through the backend's atomic write and no successful renewal reduces the stored expiry

#### Scenario: A cancellation request survives renewal

- **WHEN** cancellation was requested for the held epoch and a shorter renewal is applied
- **THEN** the result still reports cancellation and the retained maximum expiry

#### Scenario: Fencing and expired current leases

- **WHEN** an epoch is stale or no longer held
- **THEN** renewal still returns LeaseLost without changing a newer lease
- **WHEN** an expired epoch remains held and current
- **THEN** renewal may extend it under the same maximum rule

### Requirement: JSON text parsing preserves finite floating-point round trips

Cicada crates SHALL enable serde_json's `float_roundtrip` feature through their shared dependency so that serializing
and parsing a finite f64 adds no rounding error. Tests SHALL verify the feature without relying on unrelated workspace
dev dependencies. This SHALL NOT promise original JSON spelling or override a database's own numeric normalization.

#### Scenario: Numeric regression across text storage

- **WHEN** states and message payloads containing `1.0715660391465826e-75`, its negative, subnormal and finite maximum
  f64 values and boundary i64/u64 integers are written and read through each backend and copied into failure notices
- **THEN** the parser introduces no numeric change, including through subsequent dead-letter text conversion
- **AND** memory and SQLite preserve negative-zero bits; PostgreSQL's JSONB signed-zero normalization is explicitly tested and documented

#### Scenario: A downstream crate

- **WHEN** a separate consumer depends on the released Cicada crates with their normal features
- **THEN** serde_json has float_roundtrip enabled for their JSON read paths without requiring a consumer feature opt-in

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

### Requirement: Node metadata uses an additive migration

Both SQL stores SHALL append migration 7 adding nullable text hostname, cicada_version and label columns to nodes.
Existing rows SHALL retain their previous data with absent metadata. Already-open schema-6 writers SHALL keep
working with their explicit column lists; opening a newer schema with an old binary SHALL remain refused.

#### Scenario: Upgrade an existing node table

- **WHEN** a schema-6 database with node rows is opened by the new implementation
- **THEN** migration 7 preserves those rows and reads all three new fields as None
- **AND** a new heartbeat replaces all details, including clearing a previously present label
- **AND** the same details and overwrite-to-None semantics pass conformance on memory, SQLite and PostgreSQL

#### Scenario: A running old connection survives the additive migration

- **GIVEN** an open connection with prepared schema-6 node read and heartbeat statements
- **WHEN** another connection applies migration 7
- **THEN** the old prepared statements still read, insert and update using the original column lists
- **AND** old inserts leave metadata NULL, while old updates do not change existing metadata

#### Scenario: Old binaries cannot reopen or roll back

- **WHEN** a schema-6 implementation tries to open the migrated database
- **THEN** the existing newer-schema guard refuses it
- **AND** rollout instructions state that already-running old nodes may continue but cannot restart after migration
