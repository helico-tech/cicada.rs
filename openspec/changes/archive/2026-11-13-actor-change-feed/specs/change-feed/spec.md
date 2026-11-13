## ADDED Requirements

### Requirement: A host subscribes to actor change hints

ActorSystem SHALL expose changes(prefix) returning an owned ActorChanges stream of non-exhaustive ActorChange values:
Changed { address } and Resync. ActorChanges SHALL be Send + Unpin with cancel-safe next, no error items and no borrowed
system lifetime. Calling changes SHALL require a Tokio runtime, but neither a started worker nor hosted actor types.
Consumers SHALL reread Changed addresses and relist their prefix after Resync.

#### Scenario: A read-only host subscribes

- **WHEN** an unstarted ActorSystem hosting no types subscribes to a live store
- **THEN** it can receive hints and use get_actor or list_actors_with_state to refresh its view
- **AND** the first yielded item is Resync, except when the store closes before any item can be yielded

#### Scenario: A change is an invalidation hint

- **WHEN** a consumer receives Changed for an address
- **THEN** it reads the current actor and either updates its view or removes the address on None
- **AND** no state, version, operation kind or durable identity is promised by the event

#### Scenario: Recreate and repeated hints

- **WHEN** an actor is deleted and recreated before a Changed hint is processed, or duplicate hints are received
- **THEN** reading current state converges to the existing actor regardless of its reset version
- **AND** the consumer cannot infer event history or an intermediate deleted state from the hint

### Requirement: The feed covers changes of actor snapshots

Committed inserts, NewState writes, pause/resume, restart and removal including descendants SHALL be eligible for
Changed hints or coalesced Resync. A transaction SHALL collect each affected address once. Idempotent create and
absent-actor operations SHALL add no hint; repeated pause/resume MAY add a conservative hint. No Changed SHALL be
published for uncommitted or rolled-back effects.

#### Scenario: Creates and writes

- **WHEN** operator or turn creates insert actors, including children, or a turn commits NewState
- **THEN** their addresses become eligible for hints after commit, including when the new state equals its old value
- **AND** reading after a hint sees a committed value, possibly one from a still later transaction

#### Scenario: Pause and resume

- **WHEN** an operator or turn pauses or resumes a present actor
- **THEN** its address is eligible for a hint and a subsequent snapshot reflects its committed paused status
- **AND** applying the same paused status again may produce a harmless hint

#### Scenario: Removal and restart

- **WHEN** operator delete, turn delete or stop removes a parent and its descendants
- **THEN** the collector includes every removed address or coalesces the transaction to Resync
- **AND** a delete/create of one address in the same commit contributes only one address, whose read shows the new life

#### Scenario: Actor rows change without snapshot fields changing

- **WHEN** only enqueue, send, outcome, cancel, purge, deliver-now, retry, lease, claim, renew, next-due bookkeeping,
  watches, registry/node metadata or sweeps change, or a turn keeps its state without covered effects
- **THEN** no Changed is generated for those operations
- **AND** idempotent create or an operation on an absent actor adds none

#### Scenario: Refused and rolled-back effects

- **WHEN** a transaction is refused, rolled back or answered LeaseLost, or a failed handler's effects are discarded
- **THEN** it generates no Changed for those effects
- **AND** confirmed rollback without commit uncertainty generates no transaction Resync

### Requirement: Collection and delivery are bounded

A transaction collector SHALL deduplicate up to 100 canonical addresses and 7,998 total encoded payload bytes,
including @ and spaces. Exceeding either bound or lacking an affected-address set SHALL replace the batch by Resync,
without extra discovery reads. Overflow SHALL be sticky. Empty batches SHALL emit nothing. A 1024-item internal
broadcast SHALL bound queued source items; slow subscribers SHALL never block writers.

#### Scenario: Count and byte boundaries

- **WHEN** a batch has 100 unique addresses and fits 7,998 bytes, then an addition would exceed either bound
- **THEN** it becomes one Resync, releases the address collection, and stays coalesced on further additions
- **AND** duplicate addresses do not consume extra count or encoded space

#### Scenario: Large tree or unavailable address detail

- **WHEN** a transaction removes more than 100 actors or a mutation lacks an affected-address set from its existing work
- **THEN** one Resync replaces individual hints rather than adding address-discovery queries or failing the mutation

#### Scenario: A slow reader

- **WHEN** a subscriber falls behind the bounded source queue
- **THEN** it establishes a fresh subscription/readiness boundary and yields Resync when the source is live
- **AND** live hints continue afterwards without waiting for that subscriber to catch up

### Requirement: Prefixes select literal actor addresses

Changed hints SHALL be filtered by literal starts_with on the canonical address, matching the actor listing.
Resync SHALL pass every prefix. An empty prefix SHALL select all addresses. Percent encodings and underscores SHALL
be literal, not wildcards. The feed SHALL promise neither exactly-once delivery nor a total order across nodes.

#### Scenario: Prefix boundaries

- **WHEN** subscribers use empty, order, order/, a percent-containing prefix and an underscore-containing prefix
- **THEN** filtering matches the state listing; order also matches orders/x whereas order/ does not
- **AND** a source Resync reaches all those subscribers

### Requirement: Resync follows a live subscription boundary

The source SHALL serialize state changes, publication and subscription snapshots under one short hub lock. First or
recovery Resync SHALL establish a fresh receiver atomically with observing Up, discarding earlier queued hints.
Down SHALL wait for Up; Closed SHALL end. Detection and reconnection after a stale Up SHALL yield another Resync.
Consumer relists SHALL be followed by processing buffered hints; no global snapshot SHALL be promised.

#### Scenario: First LISTEN is not ready

- **WHEN** a PostgreSQL subscription is created before LISTEN succeeds
- **THEN** it yields no first item until the source becomes Up
- **AND** an unreachable database can keep it pending indefinitely, so the host may use its own timeout

#### Scenario: Changes during the resync read

- **WHEN** the stream yields Resync and the host reads the prefix page by page while writes continue
- **THEN** subsequent hints remain queued or cause a later lag Resync
- **AND** the host rereads their addresses after the listing instead of treating the pages as a global snapshot

#### Scenario: Reconnect and already queued hints

- **WHEN** a live source queues Changed, then Down, then becomes Up after reconnect
- **THEN** the queued Changed may be yielded before Down, and recovery yields Resync against the new live boundary
- **AND** hints discarded by that boundary are covered by the consumer's new full read

#### Scenario: Stale readiness is eventually corrected

- **WHEN** the source loses its connection before the listener detects it and a subscriber observes stale Up
- **THEN** that subscriber may first receive Resync, but the detected loss and successful reconnect cause another Resync
- **AND** instantaneous failure detection is not promised

#### Scenario: Polling a busy source is bounded

- **WHEN** irrelevant-prefix events or wake-ups keep arriving while a subscriber polls
- **THEN** filtering yields execution back to Tokio after bounded work instead of spinning indefinitely in one poll

### Requirement: Store close terminates the feed

Closing a store SHALL mark its hub Closed, wake pending streams, discard buffered hints and prevent listener restart.
Closed SHALL win on subsequent polls, including before a first item. Worker stop SHALL NOT close the feed. Dropping a
stream SHALL release its receiver without closing the shared listener. Cancelling next SHALL preserve stream state.

#### Scenario: Close before or during subscription

- **WHEN** Store::close runs before first poll, while a subscriber waits Down, or before changes is called
- **THEN** the stream ends with None and does not yield an initial Resync
- **AND** racing listener start cannot revive a closed store

#### Scenario: Worker and consumer lifetimes

- **WHEN** a worker stops, or one of two streams is dropped or cancels a pending next
- **THEN** the other stream remains usable, the cancelled next can be polled again, and the store remains the feed owner
- **AND** closing the store later ends all its streams, even ones with queued hints

### Requirement: Memory and SQLite publish within their shared Store scope

Memory and SQLite SHALL cover writes through the same Store backend and clones, not separate opens or processes.
Memory SHALL publish synchronously after unlocking committed state. SQLite SHALL publish after confirmed COMMIT;
uncertain commit endings with collected changes SHALL produce Resync only after settlement while the runtime remains
available. Known rollback SHALL publish no Changed.

#### Scenario: Shared handles and separate SQLite opens

- **WHEN** two systems share one Store and one changes an actor
- **THEN** the other's feed can receive the hint
- **AND** writes through a separately opened SQLite Store, even in the same process, are not reported or detected

#### Scenario: A cancelled SQLite COMMIT can still land

- **WHEN** a SQLite caller is cancelled while its COMMIT is queued or executing after changes were collected
- **THEN** a settle task retains the owned writer guard, queues rollback behind outstanding work, awaits settlement,
  publishes Resync and then releases the guard
- **AND** a resync read observes the settled outcome, with no next writer overtaking that settlement

#### Scenario: Failed COMMIT and known rollback

- **WHEN** SQLite COMMIT returns an error after collected changes, or a transaction is known to roll back before COMMIT
- **THEN** the uncertain ending resyncs after settlement, while the known rollback emits no transaction hint
- **AND** normal successful completion publishes once and does not also trigger the Drop fallback

#### Scenario: Runtime shutdown interrupts settlement

- **WHEN** the Tokio runtime cannot run or finish a SQLite settle task
- **THEN** timely post-settlement notification is not promised; existing rollback-on-next-use behaviour remains
- **AND** service resumption requires a fresh subscription/full refresh; an immediate fallback Resync is not proof
  that an outstanding COMMIT has already settled

### Requirement: PostgreSQL uses bounded transactional feed payloads

On cicada_<schema>, @ SHALL mean Resync and @ followed by canonical addresses separated by single spaces SHALL be
one change batch. All @ payloads SHALL be reserved. Validate the whole batch before yielding: malformed tokens or
spacing SHALL mean Resync. Non-@ payloads SHALL be ignored by the feed. Each transaction SHALL flush its wake-ups
and optional single feed payload together before COMMIT through one parameterized notify statement.

#### Scenario: Inter-node publication

- **WHEN** one upgraded PostgreSQL store commits covered writes and another on the same database/schema is listening
- **THEN** the latter receives the corresponding batch or coalesced Resync after commit
- **AND** the writer's own listener receives it too; rollback publishes neither actor hints nor its wake-ups

#### Scenario: Payload parsing

- **WHEN** the feed reads @, @a/b c/d, a type name, a probe or an empty payload
- **THEN** @ causes Resync, the valid batch yields matching address hints, and non-@ payloads are ignored
- **AND** @a/b bad, leading/double/trailing spaces, noncanonical encodings and other unknown @ formats cause one Resync
  without first yielding any valid prefix of the malformed batch

#### Scenario: Commit cancellation and retries

- **WHEN** a PostgreSQL commit's acknowledgement is lost or its caller is cancelled
- **THEN** notifications are delivered if and only if the server committed, subject to listener loss/lag
- **AND** each internally retried transaction uses a fresh collector; failed-attempt changes cannot leak into a later batch

#### Scenario: Ordinary wake-ups coexist with the feed

- **WHEN** a transaction has both type wake-ups and actor changes
- **THEN** the pending payloads are sent in one parameterized pg_notify/unnest statement before COMMIT
- **AND** wake-up consumers retain type matching and ignore @ payloads; ordinary wake-up traffic can consume the shared
  1024-item source capacity and trigger additional feed resyncs

### Requirement: PostgreSQL reconnect announces readiness only after LISTEN

The listener SHALL publish typed Up/Down/Payload source events. Both first connection and reconnection SHALL announce
Up only after LISTEN succeeds. PgListener eager reconnect SHALL be disabled; try_recv Ok(None) SHALL trigger the
store's reconnect loop. Wake-up streams SHALL wake on Up and lag and retain their type filtering. Listener stop SHALL
abort/join work and close the hub without leaving idle feed transactions open.

#### Scenario: Lost connection

- **WHEN** the listener's backend is terminated or sqlx signals connection loss with Ok(None)
- **THEN** it transitions Down, reconnects and performs LISTEN before announcing Up
- **AND** a feed resync after that Up is followed by hints from later covered writes, while ordinary workers are woken

### Requirement: Feed limitations and infrastructure failures are explicit

The feed SHALL add no durable history, replay or exactly-once guarantee. PostgreSQL notification failures SHALL use
ordinary store-error/retry rules; payload guards SHALL NOT be described as making commits infallible. Old writers,
direct SQL and out-of-scope SQLite writes SHALL NOT be claimed as detected. Guides SHALL describe recovery, commit
costs and optional host reconciliation outside coverage. Watches and G1–G6/S1–S10 SHALL retain their meanings.

#### Scenario: Queue exhaustion

- **WHEN** a PostgreSQL transaction that now notifies encounters a full server notification queue at COMMIT
- **THEN** COMMIT-time SQLSTATE 54000 is mapped to Unavailable, with no partial actor effects; it is not mapped to
  Invalid by the generic class-54 mapping
- **AND** other statement/error mappings and transaction retry classification remain unchanged; operations guidance
  covers notification queue usage and the extra notification cost for state-only commits

#### Scenario: Mixed writer versions

- **WHEN** older and upgraded nodes share a schema
- **THEN** old wake-up readers ignore @ payloads, but old writers' changes have no feed coverage and cause no resync
- **AND** the upgrade guidance requires all writers to upgrade and subscribers to refresh afterward

#### Scenario: A correctness-sensitive host

- **WHEN** a host needs reconciliation beyond the feed's best-effort scope or after runtime shutdown
- **THEN** its own periodic or explicit full refresh remains necessary
- **AND** it is not told that watches can observe arbitrary state writes; watches still report failure/removal only
