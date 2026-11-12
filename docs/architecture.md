# Architecture

This document is the reference for Cicada's domain language, guarantees G1–G6, store requirements S1–S10 and HTTP
contract. Decisions are recorded in the ADRs in [adr/](adr/).

Priorities, in order: simplicity, ease of use, correctness. Throughput is not a goal.

## Scope

A library embedded in Rust services. Every service that runs an `ActorSystem` is a **node**. Nodes are peers that
coordinate only through one shared transactional store (PostgreSQL, or SQLite for one host). An actor is a row of
state plus a mailbox; it occupies memory only while one message is being handled.

| Decision | Choice | Consequence |
| --- | --- | --- |
| Topology | Embedded peer nodes, one shared store | No coordinator and no cluster protocol. |
| Runtime | tokio; the HTTP API is an axum `Router` | Services that host actors are tokio services. axum supplies HTTP; the host supplies authentication. |
| Long work | Lease + heartbeat | A handler may run for minutes. A turn whose node dies is re-run elsewhere. |
| Retries | Per message, at dispatch | The sender attaches a retry policy; a failed run puts the same message back until the policy is exhausted. |
| Validation | The serializer (serde) is the only validator | JSON Schemas are generated (schemars) and published, never used to validate. |
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
| Ref | An actor's address: a type name and a key, `type/key`. A typed ref in code (`ActorRef`); its address text (`sync/acme`) on the wire, in state and in messages. The key is one name (ADR 0021): serde writes the ref as one string (a struct with one string field, or a type written as a string itself), or the ref is a unit struct or a struct that declares no field to serde, a singleton with the key `_`; a ref that writes a unit and reads a string has no address. A name is any non-empty text; in the key it keeps `A-Z a-z 0-9 . _ ~ -` and `:` and percent-encodes the rest, so a `:` separates nothing. |
| Watch | A stored pair of a target and a watcher (ADR 0018, ADR 0023). The target is one actor (`type/key`), the types under a prefix that ends at a dot (`billing` matches `billing` and `billing.invoices`, not `billings`), or every type (`*`); it need not exist, and the watch survives its delete and re-create. A turn sets and removes the watches of its own actor (`turn.watch`, `turn.unwatch`); `ActorSystem`, HTTP and the command line do so from outside. |
| Watcher | The actor a watch tells. Its message family declares `Failed(cicada::Failed)` and `Stopped(cicada::Stopped)`, or a watch of it is refused. Removing a watcher removes its watches; the watches on it as a target stay. |
| Notification | A `Failed` or a `Stopped` that the store writes into a watcher's mailbox in the transaction of the event, one per watcher and actor, marked (`notification`) and named after the event's id (`<event>.0`, `<event>.1`, …). A `Failed` says that a message to a watched actor ended `failed`; a `Stopped`, that an actor watched by its address went away (`stopped`, `deleted`, or with its `parent`). A notification that fails tells nobody. |
| Message | A typed envelope addressed to one instance, with a delivery time. |
| Mailbox | An instance's pending messages, ordered by delivery time. |
| Turn | One handling of one message: claim, load, run handler, commit. |
| Effects | What a handler produces: new state, sends, creates (independent or child), cancels, watches, stop, and control effects: pause, resume, purge and delete of another actor, and pause, resume and purge of its own (ADR 0024). Buffered until commit. |
| Commit | The single atomic write that applies a turn's effects, completes its message and writes the notifications of its event. It applies them in a fixed order: watches, outcome, cancels, deletes, creates, purges, pauses and resumes, sends, state, release of the lease, notifications. |
| Lease | A node's exclusive, time-limited right to run a turn for one instance. |
| Epoch | Per-instance counter, increased on every claim. The fencing token for commits. |
| Outcome | The terminal result of a message: `done`, `failed`, `cancelled` or `purged`. |
| Plugin | An optional extension that observes a turn through three hooks and may refuse it at claim. It never changes what a turn commits, except that it may add headers to the turn's sends. The core uses none. |

## Guarantees

| # | Guarantee |
| --- | --- |
| G1 Serial | Turns of one instance never overlap, and at most one turn commits per epoch. An instance is one life of an address, from its create to its removal: a handler of an earlier life may still run for a moment after the actor was deleted and created again, and it commits nothing (ADR 0024). |
| G2 Atomic | State change, creates, sends, watches, the control of other actors (pauses, resumes, purges and deletes), message completion and the notifications of the event (a message that ends `failed`, an actor that goes away) happen all together or not at all. |
| G3 One outcome | Every accepted message reaches exactly one outcome. None is lost; none completes twice. |
| G4 Effects once | Effects inside the system, notifications included, are exactly-once. Handler execution and external side effects are at-least-once, and repeat only after a node dies mid-turn. |
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
| S5 Fencing | `commit` and `renew` succeed only with the current epoch. |
| S6 Order | `claim_due` returns, per instance, only the pending message with the smallest `(deliverAt, seq)`, and only when it is due. |
| S7 No loss, no duplicate | A message leaves pending only by commit, cancel, purge or delete; a commit can end the pending messages of another actor, by a purge or a delete (ADR 0024). Message ids are unique across pending messages and retained outcomes. |
| S8 Liveness | An instance that is not paused, with a due head message and no valid lease, is eventually returned by `claim_due`. A paused instance is not claimed until it is resumed (ADR 0015). |
| S9 One clock | Due-ness and lease expiry use the store clock only. |
| S10 Retention | Outcomes are kept for a configured period, which is also the dedup window. A message's retention policy can hide its outcome earlier or keep it longer; a minimal record stays for the whole dedup window. |

The SQL schema (tables `actor_types`, `actors`, `messages`, `outcomes`, `watches`, plus sequences) is versioned and
migrated when a store opens a database; it is at version 5. `actors.state` is required in both stores, so a store
refuses an instance without a state. Migration 2 gave every instance without a state the state `{}` and removed the
registrations without a state schema (types that were stateless before ADR 0018). Migration 3 removed every
registration, because key fields left every contract, and rewrote `%3A` to `:` in `actors.address`,
`messages.actor_address` and `outcomes.actor_address` (ADR 0021); two old addresses that become one stop it. Migration 4
added the nullable column `actors.parent` and the index `actors_parent (parent, address)` (ADR 0019); every existing
actor is independent. It has no foreign key: the store removes a tree itself, as every removed mailbox needs its
`purged` outcomes. Migration 5 added the table `watches (target, watcher)`, both the primary key, with an index on
`(watcher, target)`, and the column `messages.notification` (ADR 0023); no existing message is a notification. Unlike
the first, these four are not additive: every node of the previous version is stopped before the first new one starts (a
node of version 3 that keeps running deletes a parent without its children; one of version 4 writes no notifications).
The SQLite store sets up only a new or empty database: one without a schema version that already holds tables belongs to
another program, and opening it fails and leaves it as it was (ADR 0017).

## Turn semantics

1. **Claim.** A node atomically takes the lease of an instance that has a due head message and no valid lease. Epoch
   and `attempt` each increase by 1.
2. **Load.** State, the own ref and the message are decoded. If the message cannot be decoded (a type name the family
   does not have, or a payload that does not fit), no handler runs: the message ends `failed` with the reason, never
   retried, and the state stays as it was.
3. **Run.** The handler runs as a future. Effects are buffered in the `Turn`. The node renews the lease by heartbeat.
4. **Commit.** One write, fenced by the epoch: watches, outcome (or reschedule), cancels, deletes, creates, purges,
   pauses and resumes, sends, state, lease release, and last the notifications of a failure or a removal.

A create in a turn is independent (`turn.create`) or makes a child of the actor that runs the turn
(`turn.create_child`). Either is idempotent and leaves an existing actor untouched, but a child create never takes
over an actor: one that exists with another parent, or with none, rejects the commit. Creates of one address in one
turn must agree on being a child; the second call fails (`TurnError::Invalid`). A stop removes the actor and every
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
sends, in the order of the calls) or from outside, where the watcher must exist (ADR 0023). When a message ends
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
| Handler returns, but its new state cannot be encoded or is JSON `null` | `failed`, error recorded, never retried | None |
| Commit rejected: a send targets an absent instance (also one the turn deletes and does not create), a child create names an actor that exists with another parent or none, or a delete names an ancestor of the turn's actor | `failed`, reason recorded | None |
| Turn exceeds its maximum duration and is cut off | As a failing handler: retried by the policy, else `failed` (`timeout`) | None |
| Handler returned, but only after its maximum duration (blocking code) | `failed` (`timeout`), never retried | None |
| Node dies or loses the lease before commit | None; message stays pending | None |
| Turn stops its instance | `done` for the turn's message; `purged` for the other pending messages of the instance and of its descendants | Applied, then the instance and its descendants are removed |
| Instance deleted, from outside or by another actor's turn, or an ancestor of it deleted or stopped | `purged` for all still-pending messages, including one being handled; a turn running on it commits nothing, and a create afterwards starts a new instance | — |
| Another actor's turn purges the instance | `purged` for every pending message except one being handled | — |
| Admin cancels a pending message | `cancelled` | — |
| Admin cancels a running turn, before its handler ended | `cancelled` at the owning node's next heartbeat, no retry; the mailbox moves on. A handler stops at its next `.await`; blocking code runs until it returns, and the commit is then `cancelled` rather than done (`on_outcome` sees it, `on_handler_error` does not run). A request after the handler ended is ignored (G6) | None |
| Admin purges a mailbox | `purged` for every pending message except one being handled | — |

Every row that ends a message `failed` tells the actor's watchers with a `Failed`, unless the message is itself a
notification; every row that removes actors tells the watchers of each removed actor with a `Stopped`.

### How the runtime maps to tokio

Each claimed turn is a task with three children: *work* (load, `on_claim`, the handler under `tokio::time::timeout`,
checks), *keeper* (heartbeat at a third of the TTL, plus a watchdog on the local lease deadline; it aborts
*work* when the lease is lost) and *overtime* (warns about handlers that do not yield). The turn task awaits *work* and
then commits while the keeper keeps renewing. Cancellation is tokio's: an aborted future stops at its next `.await`, so
a handler blocked in synchronous code keeps its turn until it returns. Blocking work belongs in `spawn_blocking`. A cancelled turn (node stop after the grace period,
or a dropped `run_due_turns` future) aborts *work*, waits for it to stop, and releases its lease only if the handler
had not ended. A claim the store answers after its caller is gone is released, never left to expire.

An admin cancel of a running turn is recorded by the store as the turn's epoch and read by the keeper at its next renewal, so it takes up to one heartbeat. The keeper aborts *work*, waits for it to stop and commits `cancelled` itself, fenced by its epoch, so G1 and G3 hold and no other node runs the message meanwhile. Whether the cancel request or the handler's end came first is decided once, by one atomic flag: a request that lost is ignored and the turn commits normally. No request can reach a later turn, because each stores the epoch it was for. A lease ends only with its turn or with its actor: no operation takes it away from a running turn (ADR 0015), and a delete, from outside or by a turn, ends it with the actor, whose message ends `purged` and is never handled again (ADR 0024).

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
| `cicada` | Addressing, definition API, runtime and dispatch loop, watches, the store (`Store` and its model; the backend trait is private), contracts and schemas, retry and retention policies, the HTTP wire model (`api`, shared by server and client). Behind features: the SQLite store (`sqlite`; sqlx, one file, WAL, one writer), the PostgreSQL store (`postgres`; sqlx, `FOR UPDATE SKIP LOCKED` claims, `LISTEN/NOTIFY` wake-ups) and the test support (`testing`: the in-memory store, clocks, `TestActors`, `claim_and_abandon`). Its tests hold the S1–S10 conformance suite every backend runs and the randomised tests: parsers, backend comparison, and a node simulation under injected faults |
| `cicada-test` | Exports the test support of `cicada` (`TestActors`, clocks, `claim_and_abandon`); contract snapshots and state samples |
| `cicada-axum` | `actor_api()`, `actor_api_read()` and `actor_health()` as axum routers, and the OpenAPI document |
| `cicada-client` | Typed HTTP client (reqwest) |
| `cicada-cli` | The binary `cicada`: commands and a terminal UI over the HTTP API, through `cicada-client`; no library crate depends on it |
| `cicada-otel` | OpenTelemetry plugin |
| `cicada-extras` | Schedules (interval, daily), crash-loop guard, failure watcher, dead letters |
| `examples` | Worked examples with tests |
| `sample-service` | A runnable node (binary) with the examples, used by end-to-end tests |
| `e2e` (`cicada-e2e`) | End-to-end and fault tests: sample nodes in Docker on PostgreSQL and SQLite, driven over HTTP (`CICADA_E2E=1`) |

## HTTP contract

See the README for the endpoints and `docs/openapi.json` for the OpenAPI document. Durations are ISO-8601 strings (`PT5S`); instants are ISO-8601 in UTC. Errors are `application/problem+json` (RFC 9457) with `type` `urn:cicada:problem:<slug>`. The routes are an
axum `Router` that the host nests into its own router and wraps in its own authentication layers. The bodies are the
types of `cicada::api`, used by both `cicada-axum` and `cicada-client`; request envelopes decode strictly (an unknown
key is `400`), payloads and states leniently.
