# Architecture

This document is the reference for Cicada's domain language, guarantees G1–G6, store requirements S1–S10 and HTTP
contract. Decisions are recorded in the ADRs in [adr/](adr/README.md); the tests that prove each guarantee are listed
in [guarantees.md](guarantees.md).

Priorities, in order: simplicity, ease of use, correctness. Throughput is not a goal.

Use this as a behavior reference. For a first implementation, start with the [core tutorial](guide/core.md).

| Look up | Section |
| --- | --- |
| Model and terminology | [Scope](#scope), [domain language](#domain-language) |
| Guarantees and backend requirements | [G1–G6](#guarantees), [S1–S10](#store-requirements) |
| Turn outcomes, failures and control effects | [Turn semantics](#turn-semantics) |
| Custom executor or worker protocol | [Remote leases](#leases-outside-the-node), [workers](#workers) |
| Wire validation and routes | [HTTP contract](#http-contract) |
| Live actor views | [Change hints](#change-hints) |
| Tradeoffs and limits | [Design notes](#design-notes), [limitations](#known-limitations) |

## Scope

A library embedded in Rust services. Every service that runs an `ActorSystem` is a **node**. Nodes are peers that
coordinate only through one shared transactional store (PostgreSQL, or SQLite for one host). An actor is a row of
state plus a mailbox; it occupies memory only while one message is being handled.

| Decision | Choice | Consequence |
| --- | --- | --- |
| Topology | Embedded peer nodes, one shared store | No coordinator and no cluster protocol. |
| Runtime | tokio; the HTTP API is an axum `Router` | Services that host actors are tokio services. axum supplies HTTP; the host supplies authentication. |
| Long work | Lease + heartbeat | A handler may run for minutes. A turn whose node dies can run again on an eligible executor, until its message reaches the claim limit. |
| Retries | Per message, at dispatch | The sender attaches a retry policy; a failed run puts the same message back until the policy is exhausted. |
| Validation | serde at the turn; the registered JSON Schema at the HTTP door | JSON Schemas are generated (schemars) and published. A send and a create over HTTP must match them on every node; a hosting node also decodes with serde, and serde stays the only judge at the turn (ADR 0026). |
| Messaging | One-way send only | A reply is an ordinary send to the sender's address. |
| Mailbox + state | One transaction boundary | State, outgoing messages and message completion commit atomically. |

## Domain language

| Term | Meaning |
| --- | --- |
| Node | A service running an `ActorSystem`. Sends messages, serves the HTTP API, and runs turns for the actor types it hosts. |
| Store | The shared durable backend: mailbox, state store, type registry, the atomic commit, and the only clock. |
| Actor type | A definition: a ref type with its type name and its message family, a state type, and a handler that takes the messages of that family. In Rust: an implementation of the `Actor` trait. A type that keeps no data has the state `NoState`, stored as `{}`. A message the type cannot decode fails; there is no policy for it (ADR 0018). |
| Contract | The published description of a type: type name, message type names, and the JSON Schemas generated from its types, the state's always among them. It holds nothing of the ref (ADR 0021). Its hash leaves out the schemas' `description`s (doc comments), so a reworded comment keeps the hash (ADR 0017). |
| Actor instance | One name of a type, explicitly created with its initial state. Owns one state value, never absent and never JSON `null`, and one mailbox (ADR 0018). One life of an address, from its create to its removal: deleted and created again, the address holds a new instance (ADR 0024). |
| Parent | The actor whose turn created an actor as a child (`turn.create_child`). Stored with the child and fixed for life; none for an independent actor (`turn.create`, or a create from outside a turn). Removing an actor, by a delete or by its own stop, removes its children, theirs, and so on: its descendants (ADR 0019). The address stays `type/key`. |
| Claim owner | The node id or worker id a claim is made for. It is a routing identity, not an authenticated machine. |
| Pin | An optional claim owner fixed for one life of an actor, chosen at creation and separate from its address and parent (ADR 0052). Only that owner may claim its turns; children do not inherit it. |
| Ref | An actor's address: a type name and a key, `type/key`. A typed ref in code (`ActorRef`); its address text (`sync/acme`) on the wire, in state and in messages. The key is one name (ADR 0021): serde writes the ref as one string (a struct with one string field, or a type written as a string itself), or the ref is a unit struct or a struct that declares no field to serde, a singleton with the key `_`; a ref that writes a unit and reads a string has no address. A name is any non-empty text; in the key it keeps `A-Z a-z 0-9 . _ ~ -` and `:` and percent-encodes the rest, so a `:` separates nothing. |
| Watch | A stored pair of a target and a watcher (ADR 0018, ADR 0023). The target is one actor (`type/key`), the types under a prefix that ends at a dot (`billing` matches `billing` and `billing.invoices`, not `billings`), or every type (`*`); it need not exist, and the watch survives its delete and re-create. A turn sets and removes the watches of its own actor (`turn.watch`, `turn.unwatch`), or watches one actor that must exist (`turn.watch_existing`, ADR 0063): the commit sets the watch when the actor is there, and otherwise writes its `Stopped` and keeps no watch; `ActorSystem`, HTTP and the command line set and remove watches from outside. |
| Watcher | The actor a watch tells. Its message family declares `Failed(cicada::Failed)` and `Stopped(cicada::Stopped)`, or a watch of it is refused. Removing a watcher removes its watches; the watches on it as a target stay. |
| Notification | A `Failed` or a `Stopped` that the store writes into a watcher's mailbox in the transaction of the event, one per watcher and actor, marked (`notification`) and named after the event's id (`<event>.0`, `<event>.1`, …). A `Failed` says that a message to a watched actor ended `failed`, and its cause; a `Stopped`, that an actor watched by its address went away (`stopped`, `deleted`, or with its `parent`). The commit of a `watch_existing` whose actor is not there also writes a `Stopped` (`deleted`, `by` null, `causedBy` the turn's message). A notification that fails tells nobody. |
| Cause | Why a message ended `failed`, in its `Failed` (ADR 0028): `handler`, `node`, `refused`, `timeout`, `message`, `state`, `commit` or `claim-limit`. The node decides it where it decides that the message fails and hands it to the store with the commit (see Turn semantics). A closed set: a new value changes the contract of every type that watches. An observer is not told it. |
| Message | A typed envelope addressed to one instance, with a delivery time. The store counts its claims (`attempt`), its failed runs that were retried (`failures`) and its lost claims (`lostClaims`). |
| Mailbox | An instance's pending messages, ordered by delivery time. |
| Turn | One handling of one message: claim, load, run handler, commit. |
| Effects | What a handler produces: new state, sends, creates (independent or child, by a typed ref or by an address), cancels, watches, stop, and control effects: pause, resume, purge and delete of another actor, and pause, resume and purge of its own (ADR 0024). Buffered until commit. |
| Commit | The single atomic write that applies a turn's effects, completes its message and writes the notifications of its event. It applies them in a fixed order: watches, outcome, cancels, deletes, creates, purges, pauses and resumes, sends, state, release of the lease, notifications. |
| Lease | A claim owner's exclusive, time-limited right to run a turn for one instance. |
| Epoch | Per-instance counter, increased on every claim. The fencing token for commits. |
| Lost claim | A claim that another claim replaced although its turn neither committed nor gave the lease back: its node died, its lease ran out, or its commit did not get through. The claim that replaces it adds 1 to the `lostClaims` of its message, which never goes back (ADR 0025). |
| Claim limit | The most lost claims a node lets a message have (`NodeConfig::max_lost_claims`, 5 by default): a message with that many ends `failed` when the node claims it, without running (ADR 0025). |
| Outcome | The terminal result of a message: `done`, `failed`, `cancelled` or `purged`. |
| Observer | Code added to a node (`TurnObserver`) that is told when each of its turns starts and, once its commit succeeded, how it ended; a turn without a commit that the node saw succeed is dropped instead. It cannot refuse a turn or change what it commits, except that it may add headers to the turn's sends (ADR 0018): to all of them, to its timers or to the others (ADR 0032). The core uses none. |
| Timer | A send of a turn to its own actor with a delivery `At` or `After`, whatever the time asked for: a zero delay or a past instant counts. The kind of send an observer's header picks (`Sends::Timers`); the send method and the actor's type do not matter, and the store keeps no mark of it (ADR 0032). |

## Guarantees

| # | Guarantee |
| --- | --- |
| G1 Serial | Turns of one instance never overlap, and at most one turn commits per epoch. An instance is one life of an address, from its create to its removal: a handler of an earlier life may still run for a moment after the actor was deleted and created again, and it commits nothing (ADR 0024). |
| G2 Atomic | State change, creates, sends, watches, the control of other actors (pauses, resumes, purges and deletes), message completion and the notifications of the event (a message that ends `failed`, an actor that goes away, an absent `watch_existing` target) happen all together or not at all. |
| G3 One outcome | Every accepted message reaches exactly one outcome. None is lost; none completes twice. |
| G4 Effects once | Effects inside the system, notifications included, are exactly-once. Handler execution and external side effects are at-least-once: a handler runs again only when its turn ended without a commit. A message whose claims were lost as often as the claim limit allows ends `failed`, whether or not its handler ever ran (ADR 0025). |
| G5 Order | Per instance, delivery follows `(deliverAt, seq)` and never happens early. |
| G6 No hidden retry | The runtime never re-runs a handler that returned. It re-runs a handler that failed only when the message's own retry policy asks for it. |

A handler that calls an external system passes `turn.message().id` as idempotency key; `attempt > 1` tells it that an
earlier run may have got part-way.

## Store requirements

The backends are PostgreSQL and SQLite, each a cargo feature of `cicada` (`postgres`, `sqlite`), and the in-memory
store of the tests (feature `testing`). A host opens one as the public type `Store` (`Store::postgres`,
`Store::sqlite`, `Store::memory`) and reads what it holds through `ActorSystem`; the backend trait is private to
`cicada`, so a store is not an extension point (ADR 0018). Every backend passes the conformance suite, test code of
`cicada` (`crates/cicada/src/tests/conformance`).

| # | Guarantee |
| --- | --- |
| S1 Durability | An acknowledged write survives process and machine crash. |
| S2 Atomicity | `commit` applies all of its parts or none. `delete`, and a commit that stops, remove the instance, its descendants and their mailboxes together. A commit that ends a message `failed`, a commit that stops and a delete write their notifications with them. |
| S3 Per-instance linearizability | All operations on one instance and its mailbox have a single order. A read after an acknowledged write sees it. |
| S4 Lease exclusivity | At most one unexpired lease per instance. Each claim issues an epoch greater than any earlier one for that address, also across delete and re-create. |
| S5 Fencing | `commit` and `renew` succeed only with the current epoch. `renew` never shortens a held lease: it keeps the later of its expiry and store now plus the TTL, atomically (ADR 0037). |
| S6 Order | `claim_due` returns, per instance, only the pending message with the smallest `(deliverAt, seq)`, and only when it is due. |
| S7 No loss, no duplicate | A message leaves pending only by commit, cancel, purge or delete; a commit can end the pending messages of another actor, by a purge or a delete (ADR 0024). Message ids are unique across pending messages and retained outcomes. |
| S8 Liveness | An instance that is not paused, with a due head message and no valid lease, is eventually returned by `claim_due` to an eligible claimer that keeps polling its type: any owner when unpinned, else its pinned owner (ADR 0052). A paused instance is not claimed until resumed (ADR 0015). |
| S9 One clock | Due-ness and lease expiry use the store clock only. |
| S10 Retention | Outcomes are kept for a configured period, which is also the dedup window. A message's retention policy can hide its outcome earlier or keep it longer; a minimal record stays for the whole dedup window. |

Claims admit only actors that are unpinned or pinned to the requesting owner, before the batch limit or any
mutation of the actor, its lease or message counters. `next_due_at` uses the same owner filter. A foreign pin does
not consume a slot or cause a busy dispatch loop; node rows and heartbeats do not affect eligibility.

The SQL schema (tables `actor_types`, `actors`, `messages`, `outcomes`, `watches`, `nodes`, plus sequences) is versioned and
migrated when a store opens a database; it is at version 8. `actors.state` is required in both stores, so a store
refuses an instance without a state. Migration 2 gave every instance without a state the state `{}` and removed the
registrations without a state schema (types that were stateless before ADR 0018). Migration 3 removed every
registration, because key fields left every contract, and rewrote `%3A` to `:` in `actors.address`,
`messages.actor_address` and `outcomes.actor_address` (ADR 0021); two old addresses that become one stop it. Migration 4
added the nullable column `actors.parent` and the index `actors_parent (parent, address)` (ADR 0019); every existing
actor is independent. It has no foreign key: the store removes a tree itself, as every removed mailbox needs its
`purged` outcomes. Migration 5 added the table `watches (target, watcher)`, both the primary key, with an index on
`(watcher, target)`, and the column `messages.notification` (ADR 0023); no existing message is a notification.
Migration 6 added the column `messages.lost_claims` (ADR 0025); no existing message has a lost claim. Unlike the first,
these five are not additive: every node of the previous version is stopped before the first new one starts (a node of
version 3 that keeps running deletes a parent without its children; one of version 4 writes no notifications; one of
version 5 counts no lost claim, and on PostgreSQL its statements that read with `*` fail on the changed table). From
version 6 on both stores name the columns they read, so a later migration that only adds a column with a default can be
additive again.
Migration 7 adds nullable `nodes.hostname`, `nodes.cicada_version` and `nodes.label`
([ADR 0048](adr/0048-node-details.md)); existing rows have none. It is additive: running schema-6 nodes keep working,
but a schema-6 binary refuses to open the migrated database, so it cannot restart or roll back. See the
[node table](operations.md#node-table) for these observability details and the limits of shared node ids. The table
feeds displays and optional [node-down alerts](guide/actors.md#node-down-alerts)
([ADR 0054](adr/0054-node-down-alerts-through-a-monitor-actor.md)); pins select eligible owners and leases fence
turns, independently of node status. Alerts neither change row retention nor turn the table into a death history.
Migration 8 adds nullable `actors.pinned_to` and the partial index `actors_pinned_due` on
`(pinned_to, type, next_due_at)` for unpaused actors with a due time, beside `actors_due` (ADR 0052). Existing actors
are unpinned. It is not safe with old direct-store nodes: an already-running schema-7 node ignores pins, and a
schema-7 binary refuses the migrated store. Stop every old direct-store node and worker before migration; see
[upgrading to 0.20.0](how-to/upgrading.md#upgrade-to-0200).
The SQLite store sets up only a new or empty database: one without a schema version that already holds tables belongs to
another program, and opening it fails and leaves it as it was (ADR 0017).

## Turn semantics

1. **Claim.** An eligible owner atomically takes the lease of an unpaused instance with a due head message and no
   valid lease. Epoch and `attempt` each increase by 1. When the instance still holds a lease that ran out and that
   nothing cleared, the claim counts it lost: the message of that lease, still pending, gets `lostClaims` + 1. A message with as many lost
   claims as the node's claim limit is not loaded or run: the node commits it `failed` (see the table). Then the node
   tells its observers that the turn starts.
2. **Load.** State, the own ref and the message are decoded. If the message cannot be decoded (a type name the family
   does not have, or a payload that does not fit), no handler runs: the message ends `failed` with the reason, never
   retried, and the state stays as it was.
3. **Run.** The handler runs as a future. Effects are buffered in the `Turn`. The node renews the lease by heartbeat.
4. **Commit.** One write, fenced by the epoch: watches, outcome (or reschedule), cancels, deletes, creates, purges,
   pauses and resumes, sends, state, lease release, and last the notifications of a failure or a removal.

A create in a turn is independent (`turn.create`) or makes a child of the actor that runs the turn
(`turn.create_child`). Either is idempotent and leaves an existing actor untouched, but a child create never takes
over an actor: one that exists with another parent, or with none, rejects the commit. Creates of one address in one
turn must agree on being a child and on the pin (`None` against `Some` included); a conflicting second call fails
(`TurnError::Invalid`) without buffering that create. An existing actor keeps its pin; delete and create starts a
new life that may choose another. The `create_with` and `create_child_with` variants take `CreateOptions`; children
are unpinned unless explicitly pinned. `turn.create_raw` and
`turn.create_child_raw` are the same creates for an address and a state given as JSON, for a turn that does not know the
type when it is compiled (ADR 0028). At the call the type of the address must be registered (`TurnError::Invalid`
otherwise; `TurnError::Store` when the registry cannot be read), and a node that hosts the type refuses an address or a
state that the type cannot read, as `create_actor` does from outside a turn. Another node cannot check them: an actor
created there with an address or a state its type cannot read fails every message with the cause `state`. A stop removes the actor and every
descendant, the children created in the same turn included, and purges their mailboxes, the turn's own sends to them
included. A turn running on a removed descendant loses its lease and commits nothing.

A turn controls other actors with the calls an operator has from outside (ADR 0024): `turn.pause`, `resume`, `purge`
and `delete`, with a typed ref, and `pause_at`, `resume_at`, `purge_at` and `delete_at`, with an address. Each means
what the same call from outside a turn means: a pause stops new turns and lets a running one finish, a purge ends every
pending message `purged` except one that is being handled, and a delete removes the actor, its mailbox and its
descendants, voids a turn running on one of them (it commits nothing, and its message ends `purged`) and tells their
watchers, with the turn's actor as `by`. No effect writes another actor's state. An effect on an actor that does not
exist does nothing and does not fail the commit, as an actor can stop itself at any moment; a send to an absent actor
still rejects it. A turn may control any actor, not only the ones it created. It may pause, resume and purge its own
actor (a purge leaves the turn's own message), but not delete it (`TurnError::Invalid` at the call; it stops itself),
and a delete of one of its ancestors rejects the commit.

Deletes come before creates, so a turn restarts an actor with a delete and a create: the old actor, its mailbox and its
descendants go, and a new one is created; an actor that is not there is only created, and nothing is removed. A parent
restarts its child with `create_child`; a restart by another actor with `create` leaves the new actor without a parent.
Creates come before purges, pauses and resumes, and those before sends, so a turn can create an actor that starts
paused, or empty a mailbox and put a first message into it. Calls of one turn on one actor have one result: the same
effect twice is applied once, of a pause and a resume the last call stands, and a delete after a create of the same
actor fails at the call. A turn that creates an actor Y and deletes an actor of which Y is a descendant ends with a new
Y, in either order of the calls: the old Y goes with the tree. When one commit removes an actor for more than one
reason, an actor the turn names in a delete reads `deleted`, the turn's own actor `stopped`, and every other `parent`;
a watcher the commit removes gets nothing, also when the commit creates it again.

A watch is set and removed by the turn of its watcher (`turn.watch`, `turn.unwatch`, applied before the creates and
sends, in the order of the calls) or from outside, where the watcher must exist (ADR 0023). The commit checks each
`watch_existing` target where it applies the watches, in the order of the calls, before its deletes and creates; on
PostgreSQL the target's row is in the commit's ordered `FOR UPDATE` set, so a concurrent delete is seen either before
the check or after the watch, and gives exactly one `Stopped` (ADR 0063). When a message ends
`failed`, the same transaction puts a `Failed` into the mailbox of every watcher of the actor's address, of a prefix of
its type and of `*`; when a turn stops its actor, or a delete from outside or by a turn removes one, a `Stopped` goes
to every watcher of the address of each removed actor, as a type target tells of failures only. A rescheduled, done or
cancelled message tells nobody, and neither does a notification that fails, so two actors that watch each other cannot
loop. A watcher that does not exist, or that the same event removes, gets nothing; a notification never fails the
commit of the watched actor. The notifications of one event are written in the order of the actor they are about and
then of the watcher.

| Situation | Message outcome | Effects |
| --- | --- | --- |
| Handler returns `Ok` | `done` | Applied atomically |
| Handler returns `Err` or panics, retry policy with attempts left, failure not permanent | None; the same message returns to pending after the backoff, `failures` + 1, `lastError` set | None |
| Handler fails (no retry left, or permanent), or state cannot be decoded | `failed`, error recorded | None |
| Message cannot be decoded (unknown type name, payload that does not fit) | `failed`, reason recorded, never retried | None |
| Handler returns, but its new state cannot be encoded (`State cannot be encoded: …`), or cannot be stored: it is JSON `null`, holds a NUL character, or is nested deeper than 64 levels (`State cannot be stored: …`, ADR 0029) | `failed`, error recorded, never retried | None |
| Commit rejected: a send targets an absent instance (also one the turn deletes and does not create), a child create names an actor that exists with another parent or none, or a delete names an ancestor of the turn's actor | `failed`, reason recorded | None |
| Turn exceeds its maximum duration and is cut off | As a failing handler: retried by the policy, else `failed` (`timeout`) | None |
| Handler returned, but only after its maximum duration (blocking code) | `failed` (`timeout`), never retried | None |
| Node dies or loses the lease before commit, or its commit does not get through | None; message stays pending, and the claim that replaces the lease counts a lost claim | None |
| Node stops in good order before the handler ended, or the answer of a claim arrives after its lease deadline (nothing runs then) | None; the lease is released, the message stays pending and the claim is not lost | None |
| A node claims a message with as many lost claims as its claim limit (ADR 0025) | `failed` (`claim limit: <n> lost claims`), no handler runs, the retry policy is not asked | None |
| Turn stops its instance | `done` for the turn's message; `purged` for the other pending messages of the instance and of its descendants | Applied, then the instance and its descendants are removed |
| Instance deleted, from outside or by another actor's turn, or an ancestor of it deleted or stopped | `purged` for all still-pending messages, including one being handled; a turn running on it commits nothing, and a create afterwards starts a new instance | — |
| Another actor's turn purges the instance | `purged` for every pending message except one being handled | — |
| Admin cancels a pending message | `cancelled` | — |
| Admin cancels a running turn, before its handler ended | `cancelled` at the owning node's next heartbeat, no retry; the mailbox moves on. A handler stops at its next `.await`; blocking code runs until it returns, and the commit is then `cancelled` rather than done (an observer is told `cancelled`). A request after the handler ended is ignored (G6) | None |
| Admin purges a mailbox | `purged` for every pending message except one being handled | — |

Every row that ends a message `failed` tells the actor's watchers with a `Failed`, unless the message is itself a
notification; every row that removes actors tells the watchers of each removed actor with a `Stopped`.

Every `failed` has exactly one cause, which its `Failed` holds (ADR 0028). The node decides it where it decides that the
message fails, and hands it to the store with the commit; the outcome and an observer get the error only:

| Way of failing | Cause |
| --- | --- |
| The handler returned an error or panicked, and no retry is left | `handler` |
| The same, but the store could not schedule the retry | `handler` |
| The handler returned an error that is not permanent and whose chain of causes holds a `TurnError::Store` or a `StoreError` | `node` |
| A read of the store the node needed failed, or a panic outside the handler | `node` |
| The handler refused the message for good, whatever its error holds | `refused` |
| The turn ran past its maximum duration, whatever the handler did | `timeout` |
| The message cannot be decoded | `message` |
| The state or the address cannot be read by the type | `state` |
| The handler returned a new state that cannot be stored (it does not encode, it is `null`, it holds a NUL character, or it is nested deeper than 64 levels) | `handler` |
| The effects of the turn were refused, or its commit was rejected or refused | `commit` |
| The claim limit | `claim-limit` |

`node` keeps a failure of the store apart from `handler`: every `send_raw` and `create_raw` reads the registry, and a
handler that passes such an error on would otherwise look like a handler that failed.

### How the runtime maps to tokio

Each claimed turn is a task. It checks the claim limit first (a message at the limit is committed `failed` and only
then are the observers told its start and its end), calls `turn_started` of each observer, and spawns three children:
*work* (load, the handler under `tokio::time::timeout`, checks), *keeper* (heartbeat at a third of the TTL, plus a
watchdog on the local lease deadline; it aborts *work* when the lease is lost) and *overtime* (warns about handlers that
do not yield). The observers' values are told the outcome after a commit succeeded, or dropped; their methods are plain
functions that run on the turn task, and a panic in one is caught and logged. The turn task awaits *work* and
then commits while the keeper keeps renewing. Cancellation is tokio's: an aborted future stops at its next `.await`, so
a handler blocked in synchronous code keeps its turn until it returns. Blocking work belongs in `spawn_blocking`. A cancelled turn (node stop after the grace period,
or a dropped `run_due_turns` future) aborts *work*, waits for it to stop, and releases its lease only if the handler
had not ended. A claim the store answers after its caller is gone is released, never left to expire, and so is a
claim whose answer arrives after its local lease deadline: nothing runs under it, and it is not a lost claim. A claim
that arrives with less than one heartbeat of its lease left runs, and its keeper renews it at once.

An admin cancel of a running turn is recorded by the store as the turn's epoch and read by the keeper at its next renewal, so it takes up to one heartbeat. The keeper aborts *work*, waits for it to stop and commits `cancelled` itself, fenced by its epoch, so G1 and G3 hold and no other node runs the message meanwhile. Whether the cancel request or the handler's end came first is decided once, by one atomic flag: a request that lost is ignored and the turn commits normally. No request can reach a later turn, because each stores the epoch it was for. A lease ends only with its turn or with its actor: no operation takes it away from a running turn (ADR 0015), and a delete, from outside or by a turn, ends it with the actor, whose message ends `purged` and is never handled again (ADR 0024).

### Leases outside the node

An executor outside the system takes turns through three operations of `ActorSystem` (ADR 0037), not through the
store: `lease_turns(owner, types, limit)`, `renew_turn(actor, epoch)` and `release_turn(actor, epoch)`. The serving
system needs no handler and may be unstarted. `lease_turns` takes 1–100 type names, each with the contract hash the
executor was built with, and claims 1–100 turns for `owner`, an id of at most 200 characters with no NUL. One read of
the registry compares every hash first, and the first type in name order that is absent or registered with another
hash refuses the request with nothing claimed; the check is not atomic with the claim, so old executors stop before a
contract is replaced. The claim uses the serving system's `lease_ttl` and claim limit and records `owner` as lease
owner: the turn shows in the running turns under that id, with no node row and no count in the local concurrency.
The requested `owner` also filters pins, never the serving system's node id; it needs no node row.
A `TurnLease` is the claim (actor, epoch, message, state, version, store times) plus the TTL the store was asked for.

- **Admission.** A message at the serving system's claim limit is never handed out: one fenced commit fails it, as a
  local node does, without a handler or the retry policy, concurrently for the batch. The notifications of its watchers
  are stored atomically with that commit, so they are there also when its acknowledgement is lost; only the observers
  wait until the commit is confirmed. Any other result, an uncertain one included, is released. The local executor
  keeps its own commit retries.
- **Deadlines.** The executor's deadline is its monotonic time when it sent the request plus the returned TTL, less
  `min(ttl / 10, 1 s)`; store times do not synchronise clocks. `lease_turns` applies the same rule to its own request
  and checks it after the claim and again after the claim limit's commits; a batch past it is not handed out.
- **Guard.** Claims stay in a guard until they are returned. A dropped future (also while the claim is in its own
  task), a system seen stopping after the claim, or a deadline that passed gives every claim not handed out one release
  attempt in the background, best effort: a closed store or an ended runtime leaves the lease to expire, and the claim
  that replaces it counts it lost. Once returned, the caller owns each lease; dropping a `TurnLease` does nothing, and
  a lost response leaves its leases to expire. No receipt is stored.
- **Renew and release.** The actor and the epoch name the lease, on any system of the store; no owner is checked, as
  these are trusted operations, not an authorization boundary. A renewal with another system's TTL keeps the later
  expiry, so a peer with a shorter TTL cannot end a lease whose holder still follows its earlier confirmed deadline,
  also when the renewal's answer is lost. It reports an operator's cancel request and ends nothing. A release leaves
  the message pending with no lost claim; repeating it, or a stale epoch, changes nothing. The holder stops its work
  before it releases and never releases a turn whose handler ended (G6). Both work after the lease ran out while no
  claim replaced it, which does not let an executor run past its deadline.
- **Life cycle.** `lease_turns` and `renew_turn` check the `NodeConfig` (`OpError::Start`) without validating or
  registering hosted types; `lease_turns` refuses a stopping or stopped system (`StoreError::Unavailable`) before the
  store, after the claim and before it hands out; renew and release work in every state. A stop does not wait for or
  release remote leases.

`complete_turn(actor, epoch, message_id, report)` ends a leased turn with what its handler did (ADR 0038), through any
system of the store, also an unstarted or stopped one or one with an invalid configuration, while the store is open.
The report is `Returned { state, effects }` (state `Unchanged`, `Set(value)` or `Stop`; effects the finished buffer of
watches, creates, sends, cancels and controls, after the executor applied `Turn`'s buffering rules),
`Failed(TurnFailure { cause, retryable, reason })` or `Cancelled`. It never names an outcome, a retry policy, a sender
or an event id: the serving system derives them, with the policy the local executor uses.

- **Policy.** Only `handler`, `node` and `timeout` failures may be retryable; `refused`, `message`, `state` and `commit`
  only not, and `claim-limit` never: any other combination is `OpError::Invalid` before the store. A retryable failure
  reads the message once; when it is pending or running for the actor, its policy and `failures + 1` decide between a
  reschedule after the capped backoff and `failed`, as for a local turn. The read is not atomic with the commit: while
  the epoch is held nothing else changes them, and the epoch fences the commit. A failed read returns the store's error
  with nothing committed. Every send gets the actor as sender and the message as cause and keeps the executor's id;
  each call draws a new event id. Reasons get NUL replaced; a reschedule's error is cut to 1,000 characters.
- **Validation.** A returned create with an out-of-bounds pin is `OpError::Invalid` before the store, leaving the
  lease held; it is not converted to a terminal failure. Otherwise the serving system checks storage limits, not types:
  a send beyond the send limits or delivery bounds, or a create whose state is `null`, has a NUL or is too deep, commits the message `failed` without effects,
  cause `commit`, `Commit refused: …`; a new own state that cannot be stored does the same with cause `handler`,
  `State cannot be stored: …`. These derived failures commit in the same call. It compiles no schema, decodes no state
  and ignores the types it hosts: registry lookups, type and state checks, watch permission, observer headers and the
  buffering rules stay the executor's duty. The store's own checks (the claimed message, a self-cancel, child and
  control conflicts, targets, ids, ancestors) still refuse a commit, some before the epoch is looked at.
- **One attempt.** Each call makes one commit attempt. `Completed` returns the outcome, the cause of a failure, the
  version (before removal for a stop), the sent envelopes and the cancelled ids, and wakes the local dispatch loop;
  `LeaseLost` answers a stale, released, completed or deleted epoch, and also a report repeated after its commit
  applied, so it proves nothing. A commit the store rejects or refuses returns `Rejected` with the failure the local
  executor would commit instead (a refused reschedule keeps the handler's cause and reason with the refusal appended),
  and commits nothing: the executor submits `Failed` with it next and never its original effects again, so a lost
  answer cannot let a peer re-evaluate effects whose rejection the executor already knows. A failure the store turns
  down is `OpError::Invalid`. Any other error may hide an applied commit: the system wakes its loop, and the executor
  repeats its current report through any system within its deadline.
- **Duties of the executor.** It decides cancellation (`Cancelled` only when the cancel request came before the
  handler ended), keeps its conservative deadline and renews, never runs a finished handler again and never releases
  after the handler ended (G6), and ends its observers only on an acknowledged `Completed`. The serving system never
  releases a lease and tells its own observers nothing. No receipt is stored: retention may hide the outcome and a
  reschedule leaves the message pending, so a later read of the message cannot always settle an uncertain completion.

### Workers

A `Worker` runs ordinary Rust actors through a `TurnSource` instead of a store (ADR 0039). The source is an
object-safe async trait over the core results of `register_contract`, `types` (as `list_types`, full `TypeDef`s),
`lease_turns`, `renew_turn`, `release_turn` and `complete_turn`, with `SourceError::{Unavailable, Invalid, Fatal}`;
`ActorSystem` and `Arc<T>` implement it. A source is trusted to keep the core's fencing, unique claims, non-shortening
renewal and explicit rejection, and not to retry a mutation behind the caller's back.

- **Placement.** Leases are filtered by `WorkerConfig::worker_id`, never the server's node id. `Turn::node_id()`
  returns that worker id, or `NodeConfig::node_id` on a node. Unpinned actors remain eligible for any owner polling
  their type; a pin whose owner never polls leaves work pending, without new attempts or automatic reassignment.
- **One executor.** The worker uses the node's `TurnExecutor` with a private holder: `Store` (a node: its TTL,
  heartbeat and claim limit, its own commit and fallback path) or `Source`. The work task ends in a `TurnReport`, the
  same for both: a node derives the decision it commits, a worker completes it. Handlers, `Turn` buffering, raw sends
  and creates (the resolver reads the registry from the store or the source, each call fresh, with the parsed-contract
  cache of ADR 0036; a source error is a store error of the turn), timeouts, decode and panic handling, typed checks
  after the handler, watches, controls, observer headers and handler-phase arbitration are shared. A worker has no
  claim limit of its own.
- **Per-turn timing.** A grant's deadline is the worker's request time plus its TTL less `min(ttl / 10, 1 s)`; a TTL
  under 2 ns or a deadline the monotonic clock cannot hold is invalid metadata. The heartbeat is `max(ttl / 3, 1 ns)` of
  the latest confirmed TTL; a renewal keeps the later deadline and, with less than a heartbeat left, the next renewal
  follows at once. Failed renewals are retried after `min(heartbeat / 4, 1 s)`. The watchdog stays independent of
  renewal calls. A worker's lease guard is strict: an extension is accepted, and a completion started, only while the
  confirmed deadline has not passed, checked under one lock, so a renewal ready with the watchdog cannot revive it.
- **Completion.** One current report and one budget (completion start plus the latest confirmed TTL, never reset).
  `Unavailable` is retried with the same report (50 ms doubling to 2 s) within the budget and the lease; each call is
  raced against both and one cut off is uncertain. `Rejected` replaces the report for good with its failure (a refused
  retry of a retryable failure included); a rejection of a non-retryable failure, or one that carries a failure nobody
  may report, is a protocol error. `Completed` ends the observers with the applied outcome and sends; `LeaseLost`,
  `Invalid`, `Fatal`, a protocol error or a spent budget abandon the turn without release.
- **Capacity and dispatch.** Slots are reserved before a lease request (at most 100, the free slots, and a manual
  call's limit) and move to the turn tasks of the batch; unused ones return. The request runs in a task of its own and
  hands a guarded batch back: a batch that nobody receives, that a stop beat to the hand-off, or that fails its checks
  (count, requested types, one lease per actor, the message's target, the TTL) releases its leases best effort. A
  malformed batch is a `Fatal` lease error. Polling repeats at once after a full batch and waits `poll_interval` or a
  turn's end otherwise; there is no due-time query or listener. Manual batches cancel a child of the worker's cancel
  token when dropped, and their tasks stay tracked.
- **Life cycle.** `NodeState` again, without a node row, sweep or heartbeat. Initialization validates the settings,
  the hosted types (at most 100) and registers each contract, requiring the stored hash; only success is cached.
  `start` registers without holding the lock a stop takes, then checks the state again. `stop` stops polling, waits
  `shutdown_cancel_timeout` for the loop, gives the tracked turns the grace period (in every state, so manual turns of
  a new worker too), cancels them and waits again. A cancelled worker turn keeps its keeper (so the watchdog bounds a
  `cancelled` completion) for at most `shutdown_cancel_timeout` while its work stops; work still running then is
  abandoned: no longer renewed, marked lost, never released, and its lease expires. A node stops renewing at once. A lease path that gets `ContractMismatch`, `Invalid` or `Fatal`
  records the first error and stops the worker from a task of its own; `stopped` returns it to every waiter, `None`
  after a requested stop. The worker never stops or closes its source.

**App workers over HTTP** (ADR 0042). `cicada-app`'s `worker` picks its mode before it resolves anything else:
`--store` the store, `--server` or `--context` HTTP, the two together a usage error; without them a non-blank
`CICADA_STORE` wins over `CICADA_SERVER` and `CICADA_CONTEXT`, and a token or the current context alone selects
nothing. Over HTTP it maps the id, concurrency, poll interval, longest turn and grace period (flag, variable, host's
`NodeConfig`, default) and the host's cancel timeout to a `WorkerConfig`, refuses the flags of the store and of the
lease policy, which the server owns, and reads none of their variables; it opens no store and no listener, and resolves
the server, token and headers as the operator commands do. The types and observers move into a private
`Hosting::{System, Worker}`, so one `App` feeds either builder; `on_start` and `App::task`, which need an
`ActorSystem`, are refused before any request. In `serve` and a worker on the store, tasks start after every
`on_start`; an early end stops the node with an error, and at a stop they share the node's grace period, with the
store closed after both (ADR 0064). The client has no redirects (a `307` would send a POST again), no retry of its own, a 30 s total
timeout and `ActorClient`'s 10 s turn timeout. Before the start it lists the types while the server answers
"unavailable", a second apart and no try after a minute; then `Worker::start` registers, and a failure there fails the
command. A stop calls `Worker::stop`; an error the worker recorded when it stopped by itself fails the command, also
when a stop came at the same time. The source is never stopped or closed.

All local timing uses `tokio::time`, so tests on a paused runtime (`start_paused = true`) run on virtual time: the
in-memory store reads its clock from `tokio::time::Instant` (`TokioClock`).

## Definition API

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sync { tenant: String }

impl ActorRef for Sync {
    const TYPE: &'static str = "sync";
    type Message = SyncMsg;   // the message family, declared once: the handler takes it
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SyncMsg { Run, FetchPage { cursor: Option<String> } }

struct SyncActor { api: ApiClient }

impl Actor for SyncActor {
    type Ref = Sync;
    type State = SyncState;

    async fn handle(&self, turn: &mut Turn<Self>, msg: SyncMsg) -> HandlerResult {
        match msg {
            SyncMsg::Run => { turn.state_mut().imported = 0; turn.send_self(SyncMsg::FetchPage { cursor: None })?; }
            SyncMsg::FetchPage { cursor } => { /* … */ }
        }
        Ok(())
    }
}
```

## Crates

| Crate | Contents |
| --- | --- |
| `cicada` | Addressing, definition API, runtime and dispatch loop with turn observers and the claim limit, watches, the store (`Store` and its model; the backend trait is private), contracts and schemas, retry and retention policies, the HTTP wire model (`api`, shared by server and client). Behind features: the SQLite store (`sqlite`; sqlx, one file, WAL, one writer), the PostgreSQL store (`postgres`; sqlx, `FOR UPDATE SKIP LOCKED` claims, `LISTEN/NOTIFY` wake-ups) and the test support (`testing`: the in-memory store, clocks, `TestActors`, `claim_and_abandon`). Its tests hold the S1–S10 conformance suite every backend runs and the randomised tests: parsers, backend comparison, and a node simulation under injected faults |
| `cicada-test` | Exports the test support of `cicada` (`TestActors`, clocks, `claim_and_abandon`); contract snapshots and state samples |
| `cicada-axum` | `actor_api()`, `actor_api_read()` and `actor_health()` as axum routers, the change hints as Server-Sent Events (`GET /changes`), the OpenAPI document, the schema check of the HTTP door (the JSON Schema validator `jsonschema`) and the sample checks |
| `cicada-client` | Typed HTTP client (reqwest) |
| `cicada-app` | Layer 2 (ADR 0018, ADR 0027): `App` turns a set of actor types into a binary with `serve` (the node and the HTTP API at the root, with a public `/ready` outside the host's router), `worker` (the node only, or a `Worker` over a server's HTTP API, ADR 0042) and the operator commands, which go over HTTP through `cicada-client`; settings from flags and `CICADA_…` variables over the host's `NodeConfig`. It only composes `cicada`, `cicada-axum` and `cicada-client`, and builds the binary `cicada` (no hosted type, so neither `serve` nor `worker`). Features `sqlite`, `postgres` and `tui` (the terminal UI), none on by default |
| `cicada-otel` | OpenTelemetry observer (`ActorTelemetry`) |
| `cicada-actors` | Layer 3 (ADR 0018, ADR 0028): ready-made actors on the public API only, each under a fixed type name: a supervisor (`cicada.supervisor`) that restarts a child at fault once per incident and escalates by failing a message of its own, dead letters (`cicada.dead-letters`) told of failures by a watch, which keep a payload deeper than 61 levels as JSON text in `payloadJson` (ADR 0043), a topic (`cicada.topic`) that delivers to each subscriber in a turn of its own, a scheduler (`cicada.scheduler`) that sends a stored message to a target on each slot of a `Schedule`, in a turn of its own, a rate limiter (`cicada.rate-limiter`) that sends each submitted message to its target at the next free slot of a shared rate, keeping one GCRA timestamp, a debouncer (`cicada.debouncer`) that keeps the latest submission of a burst and sends it once the burst is quiet, at the longest wait or on a public `Flush`, a batcher (`cicada.batcher`) that collects items in handling order and sends them to a fixed target as one message when the batch is full, at the longest wait or on a public `Flush`, a heartbeat watch (`cicada.heartbeat-watch`) that sends a stored alarm to a fixed target once when no `Beat` was handled within the allowed silence, measured from the last handled `Beat`, then waits for the next `Beat`; `Rearm` re-arms a lost timer at the unchanged deadline, and an event (`cicada.event`) that keeps a list of subscribed flow types and, for each publication, creates and starts one run of each in one commit, keyed by the publication's message id, a permit pool (`cicada.permits`, ADR 0058) that lets at most its capacity of requests hold a permit at once, queues the rest in FIFO order up to a bound, answers each `Acquire` in its commit, fences each grant with a number that only goes up, frees a delivered permit only on a `Release` of its number and frees a grant whose notice never reached its holder, a placement pool (`cicada.placement`, ADR 0067) to which nodes offer an instance, labels and places, renewed; its own self-only `Check`, armed at the earliest end of a live node's silence and kept by time and id (an early run re-arms at the same time; `Rearm` replaces a lost one), marks a quiet node silent, and each change (`silent` once per silence, `back` at the next offer, `conflict` once when another instance offers under a live node's id, whose offers are then ignored and the node marked until it goes silent or the other instance stops for one silence time, ADR 0069, or when its member reports another host name in the node's row, `Duplicate`, ADR 0071, which marks the node with that host and gives it no new places until one silence time after the last report); one live node holds a resource key an offer names (ADR 0072), and another live node offering it is ignored, listed in `contested` and alerted once; a full pool ignores a new node's offer and marks itself `full` with one `full` alert per full period (ADR 0075) goes to the pool's alert target in an `Alert` turn of its own; work asks with needs and gets `granted` (a grant number and a node: the live node with every need and the most free places, ties by id), `queued` or `refused` (`noNode`, `lineFull`, `poolFull`) in the ask's commit, the bounded line is served in order, skipping asks it cannot place now (ADR 0068), `Release` names its grant, a grant from the line is told in a `Notify` turn of its own and freed when never received, and the pool watches each holder's and waiting ask's target with `watch_existing`, so a deleted target frees its places and leaves the line, once; `PlacementMember`, a loop outside turns run as an app task, offers the node's places while the host's probe answers, under a `NodeIdentity` kept in `placement-identity.json` across restarts, which locks its folder while it lives and, each round, compares its node row's host name with the one its node writes (`NodeConfig::hostname`, `ActorSystem::hostname()`); an operator drains a node (no new places, holders kept) and declares a silent node gone, which frees its places and tells each holder with a `gone` answer in a turn of its own; and a job graph (`cicada.job-graph`, ADR 0059, ADR 0060, ADR 0061), one actor per run of a graph of jobs given as data, which starts a job once every job it needs succeeded, up to an in-flight limit, takes only a result for a running job's current attempt, skips the dependents of a failed or cancelled job, fails a job whose message failed at its watched worker or whose worker went away, cancels and retries jobs, and reports each numbered end to a target in a turn of its own; durable flows (ADR 0031), the one exception to the fixed names: a host's `Flow` names its type and is hosted by `FlowActor`, each run an actor whose state holds the input, an ordered journal (at most 1,000 `step`, `sleep`, `send` or `wait` entries), a status (running, cancelling, done, cancelled), the armed `Continue`, up to 2,000 kept replies and a cleanup journal of at most 1,000 entries (ADR 0044; 0.18.5 also executes requests, single waits and joins of up to 100 requests for all replies or the first, with optional deadlines, and `Cancel` with cleanup); each execution, only by the armed `Continue`, replays the journal and runs one new step, whose result commits with the next `Continue`, one new sleep, whose saved wake time commits with a delayed `Continue`, or one new send/request, whose optional create and message commit with its entry and the next `Continue`, enqueued once per committed entry and never converted again on replay, or one new wait, which commits closed if enough replies are already kept (all listed requests, or any for a first wait) and otherwise parks open, with a deadline `Continue` only for a timed wait; a reply turn runs no flow code and keeps only the first eligible reply from the recorded target; when its request completes the open wait, closing it with the selection (all in request order, or the listed request accepted first), cancelling any deadline timer and arming `Continue` commit with that value, even after the deadline; partial or unrelated replies keep only their values and leave the armed continuation unchanged; a `Continue` at or after the saved deadline with insufficient replies commits the timeout with any partial all selection and the next `Continue` before later flow code runs; mailbox order decides the race, a committed outcome never changes, and replay alone returns it; timeout does not stop the target; a step runs at least once with the key `<run address>/<step id>`, code outside steps must be deterministic, and a public `Continue` starts or recovers a run; a `Cancel` to a running run runs no flow code and, in one commit, cancels the armed `Continue`, marks the run cancelling and arms a cleanup `Continue`; mailbox order applies and no turn is preempted; saved status selects `Flow::on_cancel` while cancelling, over its own journal with keys `<run address>/cancel/<id>`, reading the unchanged main journal through stable lookups and waiting also on main requests no closed main wait names; replies then close only cleanup waits; only cleanup returning `Ok` after replaying every cleanup entry marks the run cancelled in its own turn; a cancelling run ignores `Cancel`, and done/cancelled runs ignore `Continue`, `Reply` and `Cancel`; `StoredMessage`, a message an actor keeps as a type name and a payload; `Schedule` (interval, daily) |
| `cicada-containers` | An optional layer for applications that run containers (ADR 0056); no library crate depends on it, which the gate checks. The `ContainerRuntime` trait (every call safe to repeat; containers, networks and volumes by name; logs in pages, bounded copies out, folders before start, a best-effort sweep by label, the runtime's machine), `ContainerSpec` with secret environment kept out of `Debug` and hardening off by default, `ContainerStatus`, `ContainerError` with one retryable kind; `OutputStore`, a store the host provides for a container's log lines, log cursor and files, written in one guarded write per page, with its input rules and, behind `testing`, `MemoryOutputStore` and `check_output_store`; `LogArchiver`, which follows the log of every container labelled with a job's prefix in rounds that never wait and keeps each page in the output store once and in order; `OrphanWatch`, which reports once a labelled container whose actor is missing in two rounds and removes nothing; `DockerRuntime` behind `docker` (bollard, no TLS; pulls with the machine's `docker login`) and `FakeRuntime` behind `testing`. On `cicada`, the reconciled container actor (ADR 0057, ADR 0062): one actor per container under a host's `ContainerJob` type name, whose goal a message sets and one armed poll at a time reconciles, and whose single end, with a failure kind, goes to a target in the commit that records it; with an output store it first collects the rest of the log and the job's output folder in its own turns, and the end reports what was kept (ADR 0065 sends progress, ADR 0066 collects) |
| `examples` | Worked examples with tests |
| `sample-service` | A runnable node (binary) on `cicada-app` with the examples, used by end-to-end tests |
| `e2e` (`cicada-e2e`) | End-to-end and fault tests: sample nodes in Docker on PostgreSQL and SQLite, driven over HTTP (`CICADA_E2E=1`) |

## HTTP contract

See [the HTTP guide](guide/http.md#http-api) for the endpoints and [openapi.json](openapi.json) for the OpenAPI document. Durations are ISO-8601 strings (`PT5S`); instants are ISO-8601 in UTC. Errors are `application/problem+json` (RFC 9457) with `type` `urn:cicada:problem:<slug>`. The routes are an
axum `Router` that the host nests into its own router and wraps in its own authentication layers. The bodies are the
types of `cicada::api`, used by both `cicada-axum` and `cicada-client`; request envelopes decode strictly (an unknown
key is `400`). The limits of an envelope and of a value come first, on every send and create, also one from code: a
field beyond its limit, a NUL character, or a payload or a state nested deeper than 64 levels is `400`
(`OpError::Invalid` from code), before the repeated id and the schema check (ADR 0029). A payload and the state of a
create are checked against the registered JSON Schema of their message type or state on every node, whether or not it
hosts the type, and refused with `422 invalid-payload` or `422 invalid-state`; a node that hosts the type then decodes
them with serde, with the same answer, and the detail says which check refused (ADR 0026). The check asks only whether a
value matches: a refusal by the schema names no place, is the same on every node, and nothing runs after it, not even
the decoder of a hosting node. A send that repeats the id of a message that is pending, or whose outcome is still shown,
is answered as a repeat before it is checked. The check asserts no `format`, loads nothing from outside the schema, and
does not enforce a schema that the validator cannot compile; JSON that code sends is not checked against the schema. No
view of a value written from 0.16.0 on is deeper than 69 levels (a page of outcomes that holds a `Failed`), so a JSON
parser that allows 128 reads every one. JSON text is parsed with exact float rounding (serde_json's `float_roundtrip`,
ADR 0043): a finite float read back has its bits, except that PostgreSQL stores a negative zero as a zero.

An HTTP create accepts optional `pinnedTo`, absent or null for unpinned, and omits it when unpinned on serialization.
The same field on a returned create of `/leases/complete` has the same rules. Pins are non-blank `NodeId`s, at most
200 characters with no NUL; invalid pins are `400`. The create response remains only `address` and `created`:
read an existing actor to see the pin it kept. Actor views always write `pinnedTo` (`null` when unpinned); new readers
accept an older server's missing field as `None`.

Actors are listed by address (`GET /actors`, `ActorSystem::list_actors`) or with their state (`GET /actors/states`,
`ActorSystem::list_actors_with_state`, ADR 0033). The second has the prefix, order, cursors and limits of the first,
and each row is the single-actor snapshot (`ActorSnapshot`, over HTTP the view of `GET /actors/{type}/{key}`). Each page
of it is one read of the store (one `SELECT` per read attempt on SQL, one lock in memory) that names its columns, so its
rows share one committed snapshot and each state comes with its own version; there is no snapshot across pages. It
returns the stored JSON without an actor or a schema, and one row that does not decode fails the page.

`GET /changes?prefix=` (ADR 0035) serves the change hints below as Server-Sent Events on both routers: `changed` with
`{"address":"<canonical>"}` and `resync` with `{}`, no `id` or `retry` field and no replay (`Last-Event-ID` is
ignored; each request is a new subscription that starts with a `resync`), and a `keep-alive` comment after 15 seconds
without output, which says nothing about readiness. The prefix is that of the listings, read with the same query
parser. The handler subscribes and answers `200` at once, without a store query; each response holds one
subscription and no queue of its own, so a slow body lags and resyncs in the core. It is the one exception to the
`503` of a closed store: closing the store ends an open body without a final event, and a request after the close
gets an empty `200`. Stopping a node leaves the streams open, so `cicada-app` bounds them with its drain (5 s after
the node stopped, then the body is cut), and a host that mounts the routers closes the store in its shutdown future
or cancels them itself.

A contract is registered by the start of a node that hosts its type, or without hosting by
`ActorSystem::register_contract` and `PUT /types/{name}` on the full router (ADR 0036), for a type that runs elsewhere.
The second works on any system, started or not, and adds no hosted type, node row or change hint; creates and sends are
accepted at once and wait until a node that hosts the type claims them. Its body is `{state, messages}` and nothing
else: the name comes from the path, and the hash is computed from the name and the schemas, never taken from the
caller. The store's atomic registration decides: an equal hash answers `200` with the stored type and its first
descriptions, another hash `409 contract-conflict` with no replacement, even without actors. Before the store, each
schema must be an object or a boolean, NUL-free and at most 120 arrays and objects deep, so that the stored contract
and its views fit parsers that allow 128 levels, and each message name obey the send limits; otherwise `422
invalid-contract`. This bound is for schemas, apart from the 64 levels of a value, and a hosting node keeps its own
rules. Schemas are not compiled: the door's fallback for an unusable schema applies later. The route is a trusted write
behind the host's authentication. A node's resolver reuses a parsed contract only when both its hash and its stored
JSON match, so descriptions that another system replaced after a removal are read fresh.

Contract hashes are `sha256:` followed by the lowercase hexadecimal SHA-256 digest of canonical UTF-8 JSON, after
removing only schema `description` keywords, recursively under keywords that hold subschemas (ADR 0017). Names and
data called `description` remain. [Canonical JSON](adr/0047-canonical-json-and-conformance-fixtures.md) sorts object keys by
UTF-8 bytes without Unicode normalization, keeps array order and writes no insignificant whitespace. Strings escape
quote and backslash, use `\b`, `\f`, `\n`, `\r`, `\t` and lowercase `\u00xx` for other controls below U+0020;
all other characters, including slash, U+007F and U+2028, remain raw UTF-8.

Represented i64/u64 numbers print as decimal integers. Finite binary64 numbers use shortest round-tripping digits,
choosing nearest digits and ties to even: decimal exponents -5 through 15 use fixed notation with `.0` for whole
floats; others use lowercase `e`, an explicit exponent sign and no leading exponent zeroes. Negative float zero
stays `-0.0`. Parsing decides the representation: `0` stays `0`, `-0` becomes `-0.0`, `1e2` becomes `100.0`, integers
outside i64/u64 can become floats, underflow can become zero, and nonfinite overflow is refused.

The fixed [fixtures](../fixtures/canonical.json) record serde_json 1.0.151 with `preserve_order` and `float_roundtrip`,
without `arbitrary_precision`, and zmij 1.0.23. Other resolved dependencies or merged features are not certified by
these finite examples. An intentional canonical-format change requires a new ADR and a hash-compatibility plan.

Remote turns (ADR 0040) are four POST routes of the full router: `/leases` (`{owner, types: {name: hash}, limit}`),
`/leases/renew` and `/leases/release` (`{actor, epoch}`) and `/leases/complete` (`{actor, epoch, messageId,
report}`), over the core operations of [Leases outside the node](#leases-outside-the-node) on any system of the store,
started or not and hosting nothing, without sessions. The actor is a canonical address in the body, so a worker echoes
it without a path to encode. Core alternatives are `200` results tagged by `result` (`leased`, `contractMismatch`,
`renewed`, `leaseLost`, `completed`, `rejected`); a release is `204`; operation errors keep the problem mapping above
(`503` for a stopping server's lease, the store's errors as everywhere). A lease carries every field of a message
(nullable ones as `null`, counters at 0 too), its state and version, the store times and the TTL, all required and the
policies decoded strictly; responses ignore unknown envelope fields. Requests refuse unknown fields at every level, and
their own parser refuses a key repeated at any depth (also an escape of it), an empty or non-object body and anything
after it, all `400` before any operation; the byte limit and serde_json's nesting limit of 128 stay, and a value within
that budget meets the core's storage checks (a too deep new state fails the turn, `completed` with cause `handler`).
`epoch`, `version` and `seq` are exact `i64` JSON integers, durations keep nanoseconds. A completion calls the core once:
no retry, fallback commit, release or observer is added by HTTP. These routes are trusted writes: an epoch is a fence,
not a secret, so the host's authentication covers `/leases` and its children and `PUT /types/{name}`.
`ActorClient` is a `TurnSource` over them and the type routes: one request per call (no use of `send_attempts`), each
of the four bounded by `turn_timeout` (default 10 s) through the response body, and errors classified by the HTTP
status alone (`400`/`413`/`422` invalid; `408`/`429`/`502`/`503`/`504`, transport and timeout unavailable; anything
else, a malformed body or an unknown variant fatal). A registration conflict reads the existing type once and computes
the requested hash from the schemas. Its `reqwest::Client` keeps its own redirect and retry policy, which a worker's
host disables.

## Change hints

`ActorSystem::changes(prefix)` (ADR 0034) is a best-effort stream of `ActorChange::Changed { address }` and
`ActorChange::Resync`, for a host that keeps a view of many actors: it reads a changed address again
(`get_actor`) and the whole prefix after a `Resync` (`list_actors_with_state`), and a `Resync` comes first. A hint
carries no state, version or kind of change, because a version starts again at 0 in an actor's next life.

- **What publishes.** A committed transaction that inserts an actor (a create from outside or by a turn, a child, a
  restart), writes a turn's new state (also an equal one), pauses or resumes one, or removes one (a delete, a turn's
  delete or a stop, with every descendant). It collects each address once, at most 100 and 7,998 encoded bytes;
  beyond either, or when it cannot name its addresses from the work it does anyway, it publishes one `Resync`. Mailbox,
  lease, watch, node and sweep work publish nothing, and neither does a rolled-back, rejected or fenced commit. The
  prefix is literal, as for the listing; a `Resync` reaches every prefix.
- **The hub.** Each store owns one bounded broadcast (1024 items) and a source state, `Up`, `Down` or `Closed`, shared
  by its clones. One short lock orders state changes, publication and a subscription with its reading of the state. A
  stream's first and every later resync takes a fresh receiver and the state together: `Up` yields `Resync`, `Down`
  waits for `Up`, `Closed` ends it. A lagging stream, a source `Resync` and a source `Up` all start such a resync, so a
  slow reader never blocks a writer. Closing the store closes the hub; it wins over queued hints. Stopping a node does
  not touch it.
- **Memory and SQLite.** They start `Up` and publish within one `Store` and its clones, right after the commit: memory
  after its lock, SQLite after a confirmed `COMMIT`. A SQLite `COMMIT` that fails, or whose caller is cancelled once it
  was asked for, may still land; with hints collected, the transaction's owned writer guard moves into a task that
  queues `ROLLBACK` behind the outstanding work, awaits it, publishes `Resync` and only then frees the writer. Without
  a running runtime no hint is promised. A separate open of the same file is not covered.
- **PostgreSQL.** A transaction adds one payload to its wake-ups on the channel `cicada_<schema>`: `@` and the
  addresses separated by one space, or `@` alone for a resync; all of them go in one `pg_notify` statement before
  `COMMIT`, so only a committed transaction is heard, and a re-run starts empty. Payloads that start with `@` belong to
  the feed: a malformed one is a resync as a whole, and wake-ups ignore them. The listener starts `Down` and turns the
  hub `Up` only after `LISTEN` succeeded, also after a reconnect (sqlx's eager reconnect is off), so no hint between a
  loss and the next `Up` goes unnoticed once the loss is seen. A `COMMIT` that fails with `54000` (a full notification
  queue) is an unavailable store. Nodes before 0.16.10 send no hints.
- **Over HTTP.** `GET /changes` carries the same items as Server-Sent Events (see HTTP contract); it adds no
  coverage, order or delivery promise.

## Design notes

- **Definitions are checked by the compiler.** One `handle` with an exhaustive `match` over the messages its ref
  names, which are declared once, on the ref (ADR 0003, ADR 0018). Handlers return `Result<(), HandlerError>`:
  `HandlerError::permanent` / `turn.fail(..)` refuse further retries, and a panic is caught and treated as a failure
  (`panic: …`).
- **The host owns the node.** The host builds an `ActorSystem`, calls `start` and `stop` itself, merges `actor_api()`
  into its router, and owns the store, or it hands all of that to `App` (`cicada-app`), which only composes those
  layers (ADR 0027). `actor_api()` takes the `ActorSystem` itself, so it cannot be mounted without a node.
- **Handlers run as futures on the tokio runtime.** A handler that does not yield is cut off only when it returns, and
  it holds a runtime worker thread meanwhile; when every worker is blocked, heartbeats and the dispatch loop stall too.
  Run blocking work in `spawn_blocking`. A handler still running well after its maximum turn or a lost lease is logged
  as a warning.
- **Validation at construction.** `DeadLettersActor::with_capacity(0)` returns an `Err` instead of failing node start.
  `RetryPolicy::new`, `Backoff::*`, `Keep::last`,
  `Keep::keep_for` and `Schedule::*` return `Result` too. A function of the core returns a named error of its module
  (`RetryError`, `RetentionError`, `IdError`, `MessageError`, `ContractError`, `DefinitionError`, `RequestError`) that
  implements `std::error::Error`, so `?` carries it into `anyhow` or an error of the host (ADR 0022).
- **Plain `Timestamp` and `MessageId` in state.** chrono's serde form is the contract's instant format, and both types
  have JSON Schemas, so states and messages hold them without attributes (issue 0701). `RetryPolicy` and `Retention`
  have JSON Schemas too (issue 0200).
- **Telemetry.** The OpenTelemetry SDK for Rust exports every span it drops, so the span of a turn whose observed value
  the node drops without an end is ended and exported with `cicada.outcome` = `abandoned` (issue 0600). The W3C
  propagator's empty `tracestate` is left out. A timer starts a new trace (ADR 0032): `cicada-otel` puts the propagator's
  headers on the other sends and, prefixed with `cicada-link-`, on timers; a turn takes its parent from the plain
  headers and at most one link from the prefixed ones, each through the configured propagator from an empty context.
  A link never becomes a parent, so each attempt of a timer is a root linked to its sender, unless the send set a plain
  parent itself. The prefix is a stable convention of `cicada-otel`; the core and the HTTP API treat these headers like
  any other, and links are best effort (a missing field, another propagator or the header limit leaves them out).
- **PostgreSQL settings.** Connection parameters go in the URL; no type of the SQL driver is in the public API, so a
  setting a URL cannot carry is added to `PostgresOptions` when a host needs it (ADR 0018). `socket_timeout` bounds a whole transaction attempt (the
  connection is then closed), not every read, because sqlx has no per-read timeout.
- **Closing a store.** Every `Store::close` call, on its first poll and before awaiting anything, refuses new operations,
  closes the change hub and stops new connections: PostgreSQL marks its main pool closed and aborts its listener;
  SQLite closes its reader permits. Each caller runs these idempotent steps; an unpolled future has no effect.
  Callers then serialize cleanup through a mutex that retains progress until completion. PostgreSQL joins the aborted
  listener, retaining its handle across a cancelled join, then closes the main pool and waits for borrowed connections.
  SQLite waits for and closes its writer (which a write or settle task may hold), then closes idle readers one at a time.
  A cancelled call leaves untouched connections for the next caller; a connection whose own close was cancelled is
  left to the SQL driver's cleanup, which later calls do not join. No caller returns merely because the closed flag
  is set, and no background task completes a cancelled close: call close again or drop the last store handle.
  SQLite does not wait for reads in flight; returning readers check the closed flag under the pool lock and drop their
  connections rather than refill a drained pool. Cleanup has no deadline. After runtime shutdown, remaining resources
  follow Tokio/sqlx drop rules without graceful close. The memory store closes immediately.
- **Cancelling `run_due_turns`.** Dropping its future cancels the turns like a node stop after the grace period: the
  worker is stopped first, then the lease is released only when the handler had not ended, and claims that arrive
  after the caller is gone are released.
- **Errors.** Error texts use Rust names (`max_lost_claims must be at least 1, or None for no limit`); a SQLite file
  of a newer schema is refused with `StoreError::Invalid`.

## Known limitations

A pin has no failover or liveness detection: if its owner never returns and polls the type, work waits forever.
Moving it requires deleting the actor and creating a new life. Duplicate executor ids do not ensure exclusive
machine placement; a pin neither authenticates a machine nor fences outside effects after a lost lease. The
ready-made supervisor, flow and event protocols cannot request pins for the actors they create (ADR 0052).

The claim limit cannot tell the message that kills a node from the messages that ran beside it, and a node killed hard
as a matter of routine (a deploy that sends SIGKILL) loses the claims of its turns too; external side effects are
at-least-once (use `message().id` as idempotency key, also across retries); a commit rejected because of the handler's
own sends is not retryable; the client has no streaming of message status (poll `GET /messages/{id}`); change hints
are best effort, with no history or replay (also over HTTP), and do not cover a separately opened SQLite file, older
PostgreSQL nodes or SQL by hand; `cicada-client` has no method for the stream of `GET /changes`.
