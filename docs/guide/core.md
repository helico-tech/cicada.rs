# The core: `cicada`

Layer 0 of Cicada (ADR 0018): actor types, the runtime, the stores, watches and the read model. Everything above it
uses only its public API. The other layers have guides of their own: [HTTP and the client](http.md), [the
app](app.md) and [the ready-made actors](actors.md). The behaviour is specified in
[the architecture](../architecture.md); how to size, deploy, upgrade and watch nodes is in
[the operations guide](../operations.md).

| Crate | Contents |
| --- | --- |
| `cicada` | Addressing (`ActorAddress`, `ActorRef`, `TypeName`), the definition API (the `Actor` trait), the runtime and dispatch loop (`ActorSystem`) with per-message retry (`RetryPolicy`) and retention (`Retention`) and the claim limit (`NodeConfig::max_lost_claims`), watches (`Turn::watch`, `Failed`, `Stopped`), the store (`Store`), turn observers (`TurnObserver`), contracts and JSON Schemas, the HTTP wire model (`cicada::api`). No HTTP server. Each store is a cargo feature, none on by default: `sqlite` (`Store::sqlite`: one file, WAL, one writer; for a single node or a few processes on one host) and `postgres` (`Store::postgres`: `FOR UPDATE SKIP LOCKED` claims, `LISTEN/NOTIFY` wake-ups; for many nodes). The feature `testing` is the test support that `cicada-test` exports. |
| `cicada-test` | `TestActors` on the in-memory store (`Store::memory`), virtual time, `claim_and_abandon` (a node that dies), contract snapshots and state samples. |
| `cicada-otel` | `ActorTelemetry`: OpenTelemetry traces and metrics per turn, an observer on the OpenTelemetry API. |

The Rust blocks of this guide are doctests of the `examples` crate (`cargo test -p cicada-examples --doc`), so they
keep compiling, and the ones that need no server or database run.

## Define an actor type

A ref struct names the type, holds the key, one name, and names the messages: an enum with `#[serde(tag = "type")]`
(the variant name is the type name on the wire). The actor's one `handle` takes that enum and matches on it, so the
compiler checks that every message is handled. A message that cannot be decoded when it is delivered (a type name the
enum does not have, or a payload that does not fit) ends `failed` with the reason, without a retry, and no handler runs.
This is [examples/src/sync.rs](../../examples/src/sync.rs), shortened:

```rust
use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sync {
    pub tenant: String, // the one name of the actor: its key
}

impl ActorRef for Sync {
    const TYPE: &'static str = "sync";
    type Message = SyncMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum SyncMsg {
    Run,
    FetchPage { cursor: Option<String> },
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SyncState {
    pub source: String,
    #[serde(default)]
    pub imported: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reports {
    pub tenant: String,
}

impl ActorRef for Reports {
    const TYPE: &'static str = "reports";
    type Message = ReportsMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ReportsMsg {
    SyncFinished { imported: i32 },
}

pub struct Page {
    pub size: i32,
    pub next: Option<String>,
}

pub trait SyncApi: Send + std::marker::Sync + 'static {
    fn fetch(&self, source: &str, cursor: Option<&str>, key: &str) -> impl Future<Output = Page> + Send;
}

/// The actor's fields are its dependencies, shared by all its turns.
pub struct SyncActor<A> {
    pub api: A,
}

impl<A: SyncApi> Actor for SyncActor<A> {
    type Ref = Sync;
    type State = SyncState;
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(5 * 60));

    async fn handle(&self, turn: &mut Turn<Self>, message: SyncMsg) -> HandlerResult {
        match message {
            SyncMsg::Run => {
                turn.state_mut().imported = 0;
                turn.send_self(SyncMsg::FetchPage { cursor: None })?;
            }
            SyncMsg::FetchPage { cursor } => {
                // External call: at-least-once, so the message id is the idempotency key.
                let page = self.api.fetch(&turn.state().source, cursor.as_deref(), turn.message().id.as_str()).await;
                turn.state_mut().imported += page.size;

                if let Some(next) = page.next {
                    turn.send_self(SyncMsg::FetchPage { cursor: Some(next) })?;
                } else {
                    let reports = Reports { tenant: turn.self_ref().tenant.clone() };
                    let imported = turn.state().imported;
                    turn.send(&reports, ReportsMsg::SyncFinished { imported })?; // the compiler checks Reports accepts it
                    // The next run; this line is the whole schedule.
                    turn.send_self_with(SyncMsg::Run, SendOptions::new().delay(Duration::from_secs(3600)))?;
                }
            }
        }
        Ok(())
    }
}
```

The address of `Sync { tenant: "acme" }` is `sync/acme`. A key is one name (ADR 0021): serde writes the ref as one
string — a struct with one string field, as here, or a type that serde writes as a string itself, such as a newtype
around a `String` — or the ref is a unit struct or a struct that declares no field to serde, a singleton with the key
`_`. A field with `#[serde(skip)]` does not count, and a ref that declares a field and writes none has no address,
nor has one that writes a unit and reads a string. A number, a boolean, a character or an enum variant is not a
string. A ref's one field cannot have a serde alias, and a
ref cannot use a reader that takes more than one form (`deserialize_any`, `from` an untagged enum): the start check
counts what the reader declares. A name may hold any text and must not be empty: in the address it keeps
`A-Z a-z 0-9 . _ ~ -` and `:` and percent-encodes the rest, so `jan@example.com` is `users/jan%40example.com` and a key
never holds a raw `/`. A `:` separates nothing. A node that hosts a type whose ref is not one string does not start,
and a send to such a ref fails at the call.

A ref with two parts joins them itself, with a separator its parts cannot hold. It can keep its typed fields and name
`String` as its serde form, as `BatchItem` in [examples/src/fanout.rs](../../examples/src/fanout.rs) does:

```rust
use cicada::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct BatchItem {
    pub batch: String,
    pub index: i32,
}

impl From<BatchItem> for String {
    fn from(item: BatchItem) -> Self {
        format!("{}.{}", item.batch, item.index)
    }
}

impl TryFrom<String> for BatchItem {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        let (batch, index) = name.rsplit_once('.').ok_or("not batch.index")?;
        Ok(Self { batch: batch.into(), index: index.parse().map_err(|_| "not an index")? })
    }
}
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# pub enum BatchItemMsg { ProcessItem }
# impl ActorRef for BatchItem { const TYPE: &'static str = "batch-items"; type Message = BatchItemMsg; }

let item = BatchItem { batch: "b1".into(), index: 7 };
assert_eq!(item.address().unwrap().as_str(), "batch-items/b1.7");
```

Inside a handler, `turn` offers:

- `self_ref()` (the typed ref), `self_address()`, `state()`, `state_mut()` and `set_state(..)` (taking the state
  mutably marks it written; it is stored at commit);
- `message()`, the envelope: `id`, `to`, `from`, `message_type`, `payload`, `attempt`, `failures`, `lost_claims`,
  `last_error`, `retry`, `retention`, `caused_by`, `deliver_at`, `correlation_id`, `headers`;
- `now()` (store time at claim; it does not move during the turn) and `current_time()` (`now` plus the time the turn
  has run so far);
- `send(&ref, msg)`, `send_with(&ref, msg, SendOptions)` and `send_self(..)` / `send_self_with(..)`. `SendOptions`
  sets `delay` or `at`, `correlation_id`, `header`, `retry`, `retention` and `id`. Each returns the new message's id.
- `send_to(&address, &msg, options).await` and `send_raw(&address, type, payload, options).await` for raw addresses
  such as `message().from`; the type name is checked against the type registry when called;
- `cancel(&id)` (a pending message of this instance gets `cancelled` at commit; keep the id a send returns to replace
  a scheduled message), `create(&ref, &initial_state)` and `create_child(&ref, &initial_state)` (applied before the
  turn's sends; see [Child actors](#child-actors)), `stop()` (removes this instance and its descendants at commit), and
  `fail(reason)`, used as `return Err(turn.fail("declined"))` (ends the message `failed` now, never retried);
- `watch(target)` and `unwatch(target)` (see [Watches](#watches)), and `pause(&ref)`, `resume(&ref)`, `purge(&ref)`
  and `delete(&ref)`, with `pause_at(&address)`, `resume_at`, `purge_at` and `delete_at` for an address (see
  [Control effects](#control-effects)).

Effects are buffered and committed atomically. A handler that returns `Err` (any error converts with `?`) or panics
commits `failed` and nothing else — unless the message carries a [retry policy](#retry). Sends refuse fields beyond
the send limits, and sends and creates a value nested too deep ([How deep a value can be](#how-deep-a-value-can-be)),
with `TurnError::Invalid`, which the handler may handle.

**Every actor has state.** A type that keeps no data has `type State = NoState`, an empty struct stored as `{}`; it is
created with `create(&ref, &NoState {})`. A state is never `null`: a create with a state that encodes to `null` (`()`,
a unit struct, a top-level `Option`) is refused (`TurnError::Invalid` in a turn, `422 invalid-state` over HTTP), and a
turn that ends with such a state ends its message `failed`, without a retry, and keeps the stored state. Use a struct.

**Times in long turns.** A `delay` counts from the commit; an `at` is absolute. In a turn that runs for 70 s,
`SendOptions::new().at(turn.now() + 1 minute)` is due 10 s *before* the commit, so it runs right after it: use
`turn.current_time() + 1 minute` (or `delay`) for a pause after the work.

**Blocking code.** Handlers run as futures on the tokio runtime. The turn timeout, a lost lease and a node stop cancel
a handler at its next `.await`; code that does not yield (blocking I/O, `std::thread::sleep`, heavy computation) runs
on until it returns and holds a runtime worker thread meanwhile. Run it with `tokio::task::spawn_blocking`.

### How deep a value can be

A payload and a state hold at most 64 arrays and objects inside each other (`cicada::runtime::limits::MAX_DEPTH`,
ADR 0029): `{"a": {"b": 1}}` is 2 deep, a number 0. A deeper value is refused like the other limits of a value:

| Where | Answer |
| --- | --- |
| A send or a create from outside a turn (`ActorSystem::send_message`, `create_actor`, the typed calls) | `OpError::Invalid`, over HTTP `400`, before the repeated id and the schema check |
| A send or a create in a turn, by a typed ref or as raw JSON | `TurnError::Invalid` at the call |
| The new state of a turn | the message ends `failed` with the cause `handler`, error `State cannot be stored: The state is nested deeper than 64 levels`, and the stored state stays |
| An operator's retry of a stored payload (`retry_message`, `POST /messages/{id}/retry`) | `RetryResult::TooDeep`, over HTTP `409 message-not-retryable`; a bulk retry skips it |

The error names the payload or the state and the limit: `The payload is nested deeper than 64 levels`. The check stops
at the first level beyond the limit, so it refuses a value of any depth with an error, and nothing after it (the store,
the decoder, a view) meets a deeper value; a `Failed` that the store writes holds the payload of the failed message
one level down, so it can be 65 deep. A value stored before 0.16.0 is delivered and decoded as before: only what is
written is checked, so a turn that writes such a state back fails ([Upgrading](../operations.md#upgrading)).

The limit leaves three things to the host:

- **A `Value` too deep for its own drop.** serde_json drops a `serde_json::Value` with a walk that calls itself, so a
  value built in code thousands of levels deep overflows the stack wherever it is dropped: in the host's code, or in
  cicada's after the refusal. A Rust value too deep for its own `Serialize` is the same. Bound the depth of what you
  build from outside data.
- **An untagged enum that holds itself.** serde decodes it by trying each variant at each level, so for a value that
  does not decode the time doubles with every level: a tree 24 levels deep, a few hundred bytes, takes seconds, and a
  decode cannot be stopped. Keep such enums out of messages and states, or make them tagged (`#[serde(tag = "…")]`).
  Over HTTP the schema check refuses most such values before they reach the decoder; not a value the schema accepts
  and only the decoder refuses (a number too large for its field), nor one below a nested message of a family that
  holds itself, which the door lets through ([The contract of a type](#the-contract-of-a-type)).
- **A type with large fields in a debug build.** The decoder of a type needs stack for each level of a value. An
  ordinary type 64 levels deep fits a thread of the runtime (2 MiB) with room to spare, but a type that holds itself
  with large fields inline (arrays of numbers, big structs) can need most of it in a debug build, where hosts run their
  tests; a release build needs far less.

## Child actors

An actor may have a **parent** (ADR 0019). `turn.create_child(&ref, &state)` creates an actor whose parent is the
actor that runs the turn; `turn.create` and every create from outside a turn (`ActorSystem::create`, `PUT`) create an
independent one. The parent is stored with the child and never changes, and the address stays `type/key`. Deleting an
actor (`delete_actor`, `DELETE /actors/{type}/{key}`, `turn.delete`) or stopping it (`turn.stop()`) removes it and
every descendant (its children, theirs, and so on) in one transaction: their pending messages end `purged`, and a turn
running on one commits nothing. Pause, resume and purge apply to one actor only.

A child create of a child of the same parent that exists already leaves it untouched, as every create does. It never
takes over an actor: one that exists as an independent actor or as a child of another one makes the commit fail, and
the turn's message ends `failed`, as for a send to an absent actor. A turn that creates one address both ways gets
`TurnError::Invalid` at the second call. The `fanout` example ([examples/src/fanout.rs](../../examples/src/fanout.rs))
creates its items as children, so deleting a batch removes the items that have not reported yet.

```rust
# use cicada::prelude::*;
# use cicada_examples::fanout::{BatchActor, BatchItem, BatchItemState};
# fn start(turn: &mut Turn<BatchActor>) -> HandlerResult {
let item = BatchItem { batch: turn.self_ref().id.clone(), index: 0 };
turn.create_child(&item, &BatchItemState { input: "x".into() })?; // removed with this batch
# Ok(())
# }
```

A turn that does not know the type of an actor when it is compiled, such as a supervisor, creates it by its address
with a state given as JSON: `turn.create_raw(&address, state)` and `turn.create_child_raw(&address, state)` behave as
`create` and `create_child`. The type of the address must be registered (`TurnError::Invalid` otherwise, and
`TurnError::Store` when the registry cannot be read). A node that hosts the type also refuses an address or a state the
type cannot read; another node cannot check them, and an actor created with such a state fails every message with the
cause `state`.

<!-- doctest
# use cicada::prelude::*;
# async fn hire<A: Actor>(turn: &mut Turn<A>) -> HandlerResult {
@@2
# Ok(())
# }
-->
```rust
let worker = ActorAddress::parse("worker/w1")?;
turn.create_child_raw(&worker, serde_json::json!({"done": 0})).await?; // checked against the registry now
```

`ActorSystem::get_actor` and `GET /actors/{type}/{key}` name the parent (`parent`, null for an independent actor);
`ActorSystem::list_children`, `ActorClient::children`, `GET /actors/{type}/{key}/children` and `cicada get children`
list the direct children in address order, one level at a time. A large tree is one large transaction; see the
operations guide, [On call](../operations.md#on-call).

## Watches

An actor can watch other actors (ADR 0018, ADR 0023): it is told, with a message in its mailbox, when a message to a
watched actor ends `failed`, and when a watched actor goes away. A watcher declares the two notifications as variants
of its message family, with the core's types `cicada::Failed` and `cicada::Stopped`, and its one handler handles them:

```rust
use cicada::prelude::*;
use serde::{Deserialize, Serialize};

/// The singleton `supervisor/_`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Supervisor;

impl ActorRef for Supervisor {
    const TYPE: &'static str = "supervisor";
    type Message = SupervisorMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum SupervisorMsg {
    Failed(cicada::Failed),   // a message to a watched actor ended `failed`
    Stopped(cicada::Stopped), // an actor watched by its address went away
    Watch { target: String }, // `billing.invoices/acme`, `billing` or `*`
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Log {
    lines: Vec<String>,
}

struct SupervisorActor;

impl Actor for SupervisorActor {
    type Ref = Supervisor;
    type State = Log;

    async fn handle(&self, turn: &mut Turn<Self>, msg: SupervisorMsg) -> HandlerResult {
        match msg {
            SupervisorMsg::Watch { target } => turn.watch(target.as_str())?,
            SupervisorMsg::Failed(f) => {
                let line = format!("{} failed {} after {} attempts: {}", f.actor, f.message_id, f.attempts, f.error);
                turn.state_mut().lines.push(line);
            }
            SupervisorMsg::Stopped(s) => {
                turn.unwatch(&s.actor)?; // done with it: a watch outlives its target
                turn.state_mut().lines.push(format!("{} went away: {:?}", s.actor, s.cause));
            }
        }
        Ok(())
    }
}
```

`turn.watch(target)` and `turn.unwatch(target)` take a typed ref, an address, a `WatchTarget` or its text. A target is
one actor (`billing.invoices/acme`), the types under a prefix that ends at a dot (`billing` matches `billing` and
`billing.invoices`, not `billings`), or every type (`*`). It need not exist, and a watch survives the delete and
re-create of its target; removing the watcher removes its watches. Both calls are buffered and applied by the commit
before its creates and sends, in the order of the calls, so the last call for a target stands; a turn that fails sets
none. `watch` fails at the call (`TurnError::Invalid`) for a text that is no target and for a family without both
variants. A family that has a `Failed` or `Stopped` of its own renames it, and adding the variants changes a type's
contract (see the operations guide, [Changing a contract](../operations.md#changing-a-contract)).

The store writes a notification in the transaction of its event, one per watcher and actor, and marks it
(`notification` in the views of a message):

- A **`Failed`** when a message ends `failed`, for the watchers of the actor's address, of a prefix of its type and of
  `*`. It holds the actor, the failed message's id, type, payload, correlation id and sender (`from`), the error and the
  attempts, and its cause, and is kept as long as the failure is: by the `failed` rule of the failed message's
  retention. A message that is retried, done or cancelled tells nobody.
- A **`Stopped`** when an actor watched by its address stops itself, is deleted, or goes with its parent: `cause` is
  `stopped`, `deleted` or `parent`, and `by` the actor whose turn removed it, `null` for a delete from outside. A watch
  on a type prefix or on `*` tells of failures only.

The **cause** of a `Failed` says why the message failed (ADR 0028). The node decides it where it decides that the
message fails:

| Cause | The message failed because |
| --- | --- |
| `handler` | its handler returned an error or panicked, and no retry is left, also when the store could not schedule the retry; or the handler returned a new state that cannot be stored |
| `node` | a read of the store the node needed failed, also one the handler passed on (an error whose chain of causes holds a `TurnError::Store` or a `StoreError`), or the node panicked outside the handler |
| `refused` | its handler refused it for good (`turn.fail`, `HandlerError::permanent`), whatever the error holds |
| `timeout` | the turn ran past its maximum duration, whatever the handler did |
| `message` | the message cannot be decoded |
| `state` | the state or the address of the actor cannot be read by its type |
| `commit` | the effects of the turn were refused, or its commit was rejected |
| `claim-limit` | it reached the claim limit |

The set is closed: a cause added later changes the contract of every type that watches, as adding the field did in
0.14.0. An observer is not told the cause.

A notification that fails tells nobody, so two actors that watch each other cannot loop; one sent again from outside
(`retry`) is an ordinary message. A watcher that does not exist, or that the same event removes, gets nothing, and the
watched actor never knows its watchers. From outside a turn, `ActorSystem::watch`, `unwatch` and `watches`, `PUT`,
`DELETE` and `GET /watches`, `ActorClient::watch`, and `cicada create watch` set, remove and list watches; the watcher
must exist, and its type must declare both variants.

## Control effects

A turn can pause, resume, purge and delete another actor (ADR 0024), with the meaning these actions have for an
operator (`pause_mailbox`, `resume_mailbox`, `purge_mailbox`, `delete_actor`): `turn.pause(&ref)` stops new turns of it
while its messages queue up and a running turn finishes, `turn.resume(&ref)` lets turns run again, `turn.purge(&ref)`
ends every pending message `purged` except one that is being handled, and `turn.delete(&ref)` removes the actor, its
mailbox and its descendants and tells their watchers, with the turn's actor as `by`. `pause_at`, `resume_at`,
`purge_at` and `delete_at` take an address, such as the `actor` of a `Stopped`. Each effect is buffered like a send and
applied by the turn's commit, with the turn's state and other effects or not at all. No effect writes another actor's
state.

To restart an actor, a turn deletes it and creates it again: deletes are applied before creates, so the old actor and
its mailbox go and a new one starts with the new state. A parent restarts its child with `create_child`, and the child
stays its child (the batch of [examples/src/fanout.rs](../../examples/src/fanout.rs)):

```rust
# use cicada::prelude::*;
# use cicada_examples::fanout::{BatchActor, BatchItem, BatchItemState};
# fn restart(turn: &mut Turn<BatchActor>) -> HandlerResult {
let item = BatchItem { batch: turn.self_ref().id.clone(), index: 0 };
turn.delete(&item)?; // the old item, its mailbox and its descendants go; a turn running on it commits nothing
turn.create_child(&item, &BatchItemState { input: "again".into() })?; // a new life, still a child of this batch
# Ok(())
# }
```

- **An actor that is not there** is left alone: the effect does nothing and the commit does not fail, because an actor
  can stop itself at any moment. So a mistyped address goes unnoticed; a typed ref makes that unlikely. A restart of an
  actor that is not there only creates it. A send to an actor that the turn deletes and does not create again still
  fails the turn's message, as a send to any absent actor does.
- **Any actor can control any other**, also one it did not create, as any actor can send anything to any other. A
  turn may pause, resume and purge its own actor, but not delete it (`TurnError::Invalid`; it calls `stop()`), and a
  delete of one of its ancestors fails its message.
- **A turn running on a deleted actor** commits nothing, and its message ends `purged`; what its handler did outside
  the store stays done. After a restart, that handler of the old life may still run for a moment next to the first
  handler of the new one, until its next heartbeat fails or its blocking code returns.
- **A child restarted by another actor** with `create` loses its descendants and has no parent; its old parent cannot
  take it back with `create_child` (that fails the parent's message). A parent restarts its own children.
- **The order in one commit** is: watches, deletes, creates, purges, pauses and resumes, sends. So a turn can watch an
  actor and delete it and be told, create an actor that starts paused, or empty a mailbox and put a first message into
  it. Calls on one actor have one result: the same effect twice is applied once, of a pause and a resume the last call
  stands, and a delete after a create of the same actor fails at the call (a restart is a delete, then a create).

The node logs each control effect of a turn it committed at the info level (see the operations guide,
[Logs](../operations.md#logs)).

## Open a store and run a node

A host opens the store, builds a node on it with the types it hosts, and starts and stops the node itself
(`ReportsActor` is from [examples/src/sync.rs](../../examples/src/sync.rs)). A host that serves the HTTP API mounts it
as in [the HTTP guide](http.md#run-it-in-an-axum-service); one that runs on `App` leaves all of this to
[the app](app.md#run-it-as-a-binary).

```rust,no_run
use cicada::store::{PostgresOptions, SqliteOptions};
use cicada::{ActorSystem, Store};
use cicada_examples::sync::ReportsActor;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = match std::env::var("DATABASE_URL") {
        Ok(url) => Store::postgres(&url, PostgresOptions::default()).await?, // feature "postgres"
        Err(_) => Store::sqlite("actors.db", SqliteOptions::default()).await?, // feature "sqlite"
    };
    let system = ActorSystem::builder(store.clone())
        .host(ReportsActor) // this node runs turns for these types
        .build();
    system.start().await?; // validates the types and registers their contracts

    tokio::signal::ctrl_c().await?; // … send with `system`, or serve the HTTP API, until Ctrl-C …
    system.stop().await; // no new claims; running turns may finish within the grace period
    store.close().await; // the store is yours: close it after the node stopped
    Ok(())
}
```

- `system.start()` validates the hosted types and registers their contracts. An invalid type, a contract that differs
  from the one already registered for its type name, fails the start with a `StartError` that lists every problem, a
  `StartProblem` each: a contract conflict is `StartProblem::ContractConflict`, with the type name and both hashes, and
  every other problem holds its sentence.
  `start` then runs the dispatch loop, the retention sweep and, on PostgreSQL, the collection of wake-ups.
- `system.stop()` stops claiming, lets running turns finish within `shutdown_grace_period` (30 s), then cancels the
  rest and releases their leases. While it stops, `/health` and `/ready` answer `503`. `stop` is idempotent.
- The store is the caller's: `ActorSystem` never closes it and does not hand it back. Open it before `start` and
  close it after `stop`; a closed store fails every operation as unavailable (HTTP `503`). A `Store` is cheap to
  clone, and clones share one backend, so several systems can run on one store. Name the store's feature on the
  dependency (`cicada = { …, features = ["sqlite"] }`, or `"postgres"`, or both); without one the crate opens no
  store. `Store::postgres(url, options)` takes the libpq form of `DATABASE_URL`
  (`postgres://user:pass@host:5432/db?sslmode=require`); every connection setting goes in the URL. TLS is built in:
  set `sslmode` on purpose ([the operations guide, TLS for PostgreSQL](../operations.md#tls-for-postgresql)).
  `Store::sqlite(path, options)` sets up a new or empty file and refuses a SQLite file that holds the tables of
  another program.
- `ActorSystem` is cheap to clone; every clone is the same node. Its methods are the in-process API: `create`,
  `send`, `send_with`, `send_to`, `send_message`, `get_actor`, `delete_actor`, `list_actors`, `pending`,
  `list_pending`, `status`, `outcome`, `cancel`, `failures`, `types`, `health`.
- The runtime logs with [`tracing`](https://docs.rs/tracing); install a subscriber (such as `tracing_subscriber`) to
  see its output.

### The contract of a type

The contract that `start` registers for a type is the JSON Schema of its state and one JSON Schema per message type,
made by schemars from the types serde decodes. `GET /types` serves it, and the HTTP door checks a send and a create
against it ([The schema check](http.md#the-schema-check)). The schema of a message type is cut out of the schema of the
family and holds every definition it refers to, whatever the name of the definition. Two cases need a word:

- **A family that holds itself** (a message `Batch { items: Vec<Msg> }`). schemars refers to the whole family with
  `"$ref": "#"`, which in the schema of one message type would mean that message. There the reference accepts any value
  (`true`): the door lets each nested message through, and the decoder checks it, at the door of a node that hosts the
  type and in the turn on every node. The schemas of the other message types do not change.
- **A hand-written schema** that refers to a definition it does not hold cannot be used by the validator: the door does
  not check that message type and warns once, and the sample checks fail with "cannot be used". A newtype variant whose
  definition is missing keeps the node from starting, with an error that names the variant and the definition; so does
  one whose reference points inside a definition (`#/$defs/Line/properties/sku`), with an error that names the variant
  and the reference, as the door may not be stricter than the decoder.

**The types that 0.15.0 changes.** Before 0.15.0 a message schema lost a definition whose name a reference escapes, and
a reference to the family meant the one message. With the fix the contracts of three kinds of type change; every other
type keeps its contract byte for byte:

- a message whose schema reaches a type whose name has a space, `/`, `~`, `<`, `>`, another character outside the safe
  set of an address, or a letter that is not ASCII, from `#[schemars(rename)]`, a container `#[serde(rename)]` or the
  Rust name (`Größe`), at any depth (a field of that type, or a field of a type `A` that holds one): its schema gains
  the definition, so the door now checks that message type;
- a family that holds itself: the schemas that refer to the family get `true` there, so the door no longer refuses a
  valid nested message;
- a hand-written schema with a reference of several segments (`#/$defs/Line/properties/sku`): its message schema now
  keeps `Line`.

A newtype variant of a type whose name a reference escapes kept the node from starting ("must be an enum with
`#[serde(tag = "type")]`"); it starts now, with its first contract. To tell whether a type is hit: its
[contract snapshot](#contract-snapshots) changes, or a node of 0.15.0 refuses to start against the registration of the
release before with `StartProblem::ContractConflict`. A type that is hit is deployed as any change of a contract: under
a versioned type name, or with a stop of the world that deletes its actors and its registration
([Changing a contract](../operations.md#changing-a-contract)). The operations guide has the steps of the upgrade
([Upgrading](../operations.md#upgrading)).

### Configuration

Node settings are the fields of `NodeConfig`, passed to the builder:
`ActorSystem::builder(store).config(NodeConfig { lease_ttl: Duration::from_secs(60), ..NodeConfig::default() })`, or
to `App::node_config`. A binary built from `App` also takes most of them as flags and `CICADA_…` variables
([Run it as a binary](app.md#run-it-as-a-binary)).

| Setting | Default | Meaning |
| --- | --- | --- |
| `node_id` | random | Lease owner, shown in `/messages/{id}` while running and by `/health`. |
| `lease_ttl` | 30 s | A turn's lease; a dead node's turn is re-claimed after it expires. |
| `heartbeat_interval` | `lease_ttl / 3` | Lease renewal; must be in `(0, lease_ttl / 2]`. |
| `poll_interval` | 500 ms | Longest sleep of the dispatch loop (wake-ups usually come sooner: own sends, PostgreSQL `NOTIFY`). |
| `default_max_turn` | 15 min | Turn timeout (`failed("timeout")`) unless the type sets `MAX_TURN`. |
| `concurrency` | 16 | Turns running at once on this node. |
| `shutdown_grace_period` | 30 s | Running turns may finish on stop; then they are cancelled and their leases released. |
| `shutdown_cancel_timeout` | 5 s | How long a cancelled turn may take to end before it is left to its lease. |
| `sweep_interval` | 1 h | Deletion of expired outcomes. |
| `max_lost_claims` | `Some(5)` | The [claim limit](#claim-limit): a message with this many lost claims ends `failed` without running; `None` turns it off, `Some(0)` fails the start. Set one value on every node. |

Handlers run on the tokio runtime that called `start`; there is no separate handler pool (see
[Blocking code](#define-an-actor-type)).

Store settings: outcome **retention 7 days** (`SqliteOptions::retention`, `PostgresOptions::retention`), also the
dedup window. SQLite: `SqliteOptions { clock, retention, busy_timeout: 5 s }`. PostgreSQL:
`PostgresOptions { schema: "public", retention, max_pool_size: 20, socket_timeout: 30 s }` (deadlocks and
serialization failures are re-run). [The operations guide](../operations.md#sizing) says how to size them.

## Retry

Retry is part of a message's **delivery**, set by its **sender** next to the delivery time — like a workflow engine's
caller choosing the retry policy of an activity. Actor types and observers do not configure it, and a message sent
without a policy is never retried: a handler that fails ends it `failed`.

<!-- doctest
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada_examples::saga::{Order, OrderActor, PaymentGateway, Payments, PaymentsActor, PaymentsMsg};
@@1
# fn on_reserved(turn: &mut Turn<OrderActor>, me: Order, amount_cents: i64) -> HandlerResult {
@@4
# Ok(())
# }
@@2
# async fn on_charge<G: PaymentGateway>(
#     turn: &mut Turn<PaymentsActor<G>>, gateway: &G, reply_to: Order, amount_cents: i64,
# ) -> HandlerResult {
@@5
# let _ = result;
# Ok(())
# }
# fn main() {}
-->
```rust
// Sender (the Order saga, examples/src/saga/order.rs): this delivery gets up to 4 handler runs.
let backoff = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(30)) // 1 s, 2 s, 4 s, … or Backoff::fixed
    .expect("a valid backoff");
let retry = RetryPolicy::new(4, backoff).expect("a valid policy"); // handler runs, including the first
turn.send_with(&Payments, PaymentsMsg::Charge { reply_to: me, amount_cents }, SendOptions::new().retry(retry))?;

// Receiver (Payments): an error is retried under the sender's policy; turn.fail(..) is a veto.
if amount_cents <= 0 {
    return Err(turn.fail(format!("invalid amount {amount_cents}"))); // permanent: failed now, never retried
}
// A retry runs the same message again, so its id is stable across attempts: the right idempotency key.
let result = gateway.charge(&reply_to.id, amount_cents, turn.message().id.as_str()).await?; // Err: retried
```

Every send takes `retry` through `SendOptions::retry`: `send_with` in a handler (typed and raw), the `ActorSystem`
sends, `ActorClient::send_with` / `send_message`, `TestActors::send_with`, and over HTTP the `retry` field of
`POST /actors/{type}/{key}/messages`:

```sh
curl -X POST localhost:8080/actors/payments/messages -d '{"type": "Charge", "payload": {…},
  "retry": {"maxAttempts": 4, "backoff": {"kind": "exponential", "initial": "PT1S", "factor": 2.0, "max": "PT30S"}}}'
# or "backoff": {"kind": "fixed", "delay": "PT5S"}; durations as ISO-8601; factor defaults to 2
```

Bounds: `max_attempts` in 1..100; every duration positive and at most `cicada::time::MAX_DELAY` (about 1000 years);
`max >= initial`; `factor` finite and ≥ 1. `RetryPolicy::new` and `Backoff::exponential` / `Backoff::fixed` return an
`Err` for a policy outside them, and HTTP answers `400 bad-request`. There are no error filters (a policy is data,
stored with the message) and **no jitter** (delays are deterministic, which keeps virtual-time tests and `deliverAt`
exact).

**What happens.** When a handler run fails (it returns `Err`, panics, or exceeds its maximum turn), the message has a
policy, the failure is not permanent and `failures + 1 < max_attempts`, the dispatcher puts the **same message** back
into the mailbox, in the same commit: same id, type, payload, `from`, `correlation_id`, `headers` and `retry`, due
after `backoff.delay(failures + 1)`, with a new `seq` (it queues again behind messages already due), `failures` + 1
and `last_error` = the handler's error (at most 1000 characters). It gets no outcome in between: `GET /messages/{id}`
shows `pending` with `failures`/`lastError`, and the message ends with exactly one outcome — `done` when an attempt
succeeds, `failed` with the handler's error when the policy is exhausted. `/outcomes?status=failed` lists only such final failures.
The handler's buffered effects are discarded on every failed run, as for any failure. Cancel works on a waiting
message; deleting or stopping the instance purges it.

- **The receiver's veto.** A handler that returns `HandlerError::permanent(reason)` (or `turn.fail(reason)`, the same
  thing) ends the message `failed` at once with that reason, whatever the policy. `HandlerError::permanent_from(err)`
  keeps the cause.
- **Not retried:** undecodable state, a message that cannot be decoded (no handler runs; it ends `failed` with the
  reason), a message at the [claim limit](#claim-limit), and commits rejected by the store (e.g. a send to an absent
  instance).
- **Timeouts.** A run cut off at its maximum turn is a failure retried per policy. A handler that blocks its thread
  past the maximum (code that does not yield) is waited for; it is retried only if it then fails with an ordinary
  error. If it returns `Ok` or vetoes, the message ends `failed` with `timeout`, never re-run.
- **Counting.** `message().failures` counts failed runs (this run is attempt `failures + 1`); `message().attempt`
  counts claims, including crash recoveries, which do not use up an attempt; `message().lost_claims` counts the claims
  that were lost, which the [claim limit](#claim-limit) caps.
- **Forwarding.** `message().retry` is the incoming policy; pass it on with
  `SendOptions { retry: turn.message().retry.clone(), ..SendOptions::new() }`.
- **Dedup.** A resend with an id that was already accepted is `200` / `SendResult::Duplicate` when target and type
  match, as before; a different `retry` on it is ignored, like a different payload — the first accepted send stands.
- **Observers.** An [observer](#observers) is told the end of each retried run as `TurnOutcome::Rescheduled`, with the
  error and the delivery of the retry, and the end of the last as `failed`. It observes only.
- **Unreadable policies.** A stored or served policy that a node or client cannot decode (a backoff kind of a newer
  version, a damaged row) is logged as a warning and read as no policy: the message is handled without retry, and
  claims, status reads and listings keep working.
- A backoff never exceeds `MAX_DELAY`; a retry the store still cannot schedule (due past `cicada::time::MAX_INSTANT`)
  ends `failed` with the handler's error and the refusal.

`RetryPolicy` and `Retention` have JSON Schemas, so a message or state may carry one as a field.

## Claim limit

A message that kills its node (out of memory, `std::process::exit`, an abort) would otherwise be claimed again for
ever, and take a node down each time. The core caps it (ADR 0025): a message counts its **lost claims**, and a node
that claims a message with as many lost claims as its limit, 5 by default, does not run it. It commits the message
`failed` with the error `claim limit: <n> lost claims`, without asking its retry policy, and its watchers are told with
a `Failed`, as for any failure.

A claim is lost when another claim takes its place although its turn neither committed nor gave the lease back: its
node died or was killed, its lease ran out (a long pause, a partition), or its commit did not get through. The claim
that replaces such a lease counts it, in the message's `lost_claims` (`lostClaims` over HTTP). A node that stops in good
order and gives its claims back loses none, and neither does a failed run that is committed as a retry, or a claim
whose answer a slow store gave only after its lease deadline: the node gives it back without running anything. The
count never goes back, also not after a retry; a message sent again (`POST /messages/{id}/retry`) is a new message with
none.

<!-- doctest
# use cicada::ActorSystem;
# use cicada::runtime::NodeConfig;
# fn node(store: cicada::Store) {
@@5
# }
-->
```rust
let config = NodeConfig {
    max_lost_claims: Some(10), // None turns the limit off
    ..NodeConfig::default()
};
let system = ActorSystem::builder(store).config(config).build();
```

The limit cannot tell the message that kills a node from the messages that ran beside it: each of them loses its claim
when the node dies, and they all reach the limit together. Their errors name the limit, so they can be sent again; the
one that fails again is the cause. A watcher can tell such a failure from a failure of a handler by the start of the
error, `claim limit:`, though a handler can return an error that starts the same.
[The operations guide](../operations.md#the-claim-limit) says what counts in practice and how to choose a limit.

## Message persistence

How long a finished message stays readable is also part of its delivery, chosen by the sender next to `retry`: a
`Retention` policy with one rule for a message that ends `done` and one for a message that ends `failed`. A missing
rule, and the outcomes `cancelled` and `purged`, keep the store default (the store's retention, 7 days by default).
The block sends to `Payments` of [examples/src/saga](../../examples/src/saga/):

<!-- doctest
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada_examples::saga::{Order, Payments, PaymentsMsg};
# async fn example(system: &ActorSystem, order: Order) -> anyhow::Result<()> {
@@9
# Ok(())
# }
-->
```rust
let charge = || PaymentsMsg::Charge { reply_to: order.clone(), amount_cents: 900 };
let gone_when_done = SendOptions::new().retention(Retention::REMOVE_ON_DONE); // gone once it succeeded
system.send_with(&Payments, charge(), gone_when_done).await?;

let retention = Retention {
    done: Some(Keep::last(100).expect("a valid rule")), // the 100 most recent done charges of this instance
    failed: Some(Keep::keep_for(Duration::from_secs(90 * 24 * 3600)).expect("a valid rule")), // 90 days
};
system.send_with(&Payments, charge(), SendOptions::new().retention(retention)).await?;
```

```sh
curl -X POST localhost:8080/actors/mailers/m-1/messages -d '{"type": "Send", "payload": {…},
  "retention": {"done": {"kind": "remove"}, "failed": {"kind": "for", "age": "P30D"}}}'
# rules: {"kind": "remove"} | {"kind": "for", "age": "PT1H"} | {"kind": "last", "count": 100}
```

- **`Keep::Remove`** hides the record as soon as the message finishes and drops its payload, headers, correlation id
  and error at once.
- **`Keep::For { age }`** keeps it readable for `age` after it finished, shorter or longer than the store retention.
- **`Keep::Last { count }`** keeps it while it is among the `count` newest records of the same instance and outcome
  that also chose `Last` (and at most for the store retention). Records with another rule are never trimmed by it.
- A removed record is gone from `GET /messages/{id}` (404), `/outcomes` and cancel. Its content is dropped at once
  (`Remove`) or by the next retention sweep (`sweep_interval`, 1 h by default).
- **Dedup keeps working.** The store keeps a minimal record (id, target, sender, type, status, times, cause) for
  its retention, which is also the dedup window: a resend with a removed message's id is still `200`, so a client
  retry after a lost acknowledgement never runs the message twice.
- Bounds: `count` in 1..10 000; `age` positive and at most `MAX_DELAY`; `Keep::last` and `Keep::keep_for` check them.
  Invalid policies are `400` over HTTP. A resend of an accepted id with another policy is a duplicate; the first
  policy stands. Unreadable stored policies read as none. Every send path takes `retention`, as for `retry`;
  `message().retention` lets a handler pass it on.

## Message causes

Every message sent from a turn records the id of the message that turn handled as `caused_by` (`causedBy` over HTTP;
`None` for a send from outside an actor). It is part of the envelope everywhere a message is served, so a tool can
follow a message back to what started it, one `GET /messages/{id}` at a time. `GET /messages/{id}/caused` goes the
other way, one level down; a tool that wants the tree calls it again for each child.

## Timers

A timer is a delayed self-send. To stop or move it, keep the id of the pending message in state and
cancel it with `turn.cancel`: the cancel, the state change and any new timer commit together, and the turns of one
actor never overlap, so state and mailbox never disagree. There is no timer type; this is the whole recipe
([examples/src/timer.rs](../../examples/src/timer.rs) is the full version):

<!-- doctest
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada::store::MessageId;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Checkout { id: String }
# impl ActorRef for Checkout { const TYPE: &'static str = "checkout"; type Message = CheckoutMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum CheckoutMsg { Begin, Pay, PaymentTimeout }
# #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
# struct CheckoutState { timeout: Option<MessageId>, paid: bool, expired: bool }
# struct CheckoutActor;
# impl Actor for CheckoutActor {
#     type Ref = Checkout;
#     type State = CheckoutState;
#     async fn handle(&self, turn: &mut Turn<Self>, msg: CheckoutMsg) -> HandlerResult {
@@17
#         Ok(())
#     }
# }
-->
```rust
match msg {
    CheckoutMsg::Begin => {
        let options = SendOptions::new().delay(Duration::from_secs(15 * 60));
        let timeout = turn.send_self_with(CheckoutMsg::PaymentTimeout, options)?;
        turn.state_mut().timeout = Some(timeout); // keep the id, to cancel it later
    }
    CheckoutMsg::Pay => {
        if let Some(timeout) = turn.state_mut().timeout.take() {
            turn.cancel(&timeout)?; // gone in the same commit as the state change
        }
        turn.state_mut().paid = true;
    }
    CheckoutMsg::PaymentTimeout => {
        turn.state_mut().timeout = None;
        turn.state_mut().expired = true;
    }
}
```

A cron actor arms its next tick the same way; `Schedule` of the ready-made actors gives it the next slot (see
[Schedule](actors.md#schedule)), and the ready-made [scheduler](actors.md#scheduler) sends a stored message on each slot
with no host code.

## Observers

An observer watches the turns of a node start and end, and has no say in them (ADR 0018). It implements
`TurnObserver`: `turn_started` is called once a message is claimed, before anything is decoded or run, and returns an
`ObservedTurn`, whose `turn_ended` is told the committed outcome and the messages the commit sent once the commit
succeeded. A value that is dropped without that call means that the node saw no commit of the turn succeed: its lease
was lost or ran out, its node stopped, its commit did not get through, or its actor was deleted. An observer that cares
keeps a flag and implements `Drop`. Both are plain functions that run in the turn, so they must not block.

An observer cannot refuse a turn or change what it commits. It may add headers to the messages the turn sends
(`TurnStart::add_send_header`), which is how tracing travels, and a header never fails a send: one that the limits of a
message refuse is left out when it is added, and a send takes its own headers first and then those of the observers, in
the order of their names, as long as the message stays within 64 headers. A send's own header keeps its value. A panic
in an observer is caught and logged, and the turn goes on as if that observer were not there. The handler reads nothing
an observer made.

<!-- doctest
@@2
# use cicada::ActorSystem;
# use cicada_examples::sync::ReportsActor;
@@22
# fn node(store: cicada::Store) {
@@4
# }
-->
```rust
use cicada::observer::{ObservedTurn, TurnEnd, TurnObserver, TurnStart};
use cicada::store::MessageId;

struct Audit {
    prefix: String,
}

struct AuditedTurn {
    prefix: String,
    message: MessageId,
}

impl TurnObserver for Audit {
    fn turn_started(&self, turn: &mut TurnStart<'_>) -> Box<dyn ObservedTurn> {
        Box::new(AuditedTurn { prefix: self.prefix.clone(), message: turn.message().id.clone() })
    }
}

impl ObservedTurn for AuditedTurn {
    fn turn_ended(self: Box<Self>, end: &TurnEnd<'_>) {
        println!("{}: {} -> {:?}", self.prefix, self.message, end.outcome());
    }
}

let system = ActorSystem::builder(store)
    .observe(Audit { prefix: "node".into() }) // for every hosted type
    .host(ReportsActor)
    .build();
```

A node takes any number of observers and calls them in the order they were added, at the start and at the end; of two
that add a header of one name, the one added later wins. Pass an `Arc` to keep a handle. `()` is an `ObservedTurn` that
is told nothing, for an observer that only adds headers. The table of which call comes in which situation is in the
`cicada::observer` docs.

### OpenTelemetry

`cicada-otel` adds OpenTelemetry tracing and metrics as an observer. It depends on the OpenTelemetry API only; the
service brings the SDK and exporters (`opentelemetry_sdk`, `opentelemetry-otlp`) and installs them as the global
providers, or passes them to `ActorTelemetry::new(&tracer_provider, &meter_provider)`:

<!-- doctest
# use std::sync::Arc;
# use cicada::{ActorSystem, SendOptions};
# use cicada_examples::sync::{Sync, SyncMsg};
@@1
# async fn node(store: cicada::Store) -> anyhow::Result<()> {
@@5
# Ok(())
# }
-->
```rust
use cicada_otel::{ActorTelemetry, current_trace_headers};

let system = ActorSystem::builder(store).observe(ActorTelemetry::global()).build(); // the global providers
// Outside an actor, the caller's trace context goes into the message's headers:
let options = SendOptions { headers: current_trace_headers(), ..SendOptions::new() };
system.send_with(&Sync { tenant: "acme".into() }, SyncMsg::Run, options).await?;
```

- **A span per turn**, `process <type name>` (kind `CONSUMER`), from the claim to the commit, with `messaging.*` and
  `cicada.*` attributes (actor, message type, attempt, failures, outcome, messages sent). `failed` turns have status
  `ERROR`, and a turn that ends `failed` or is rescheduled has an `exception` event with the error. A turn of which the
  node saw no commit succeed (lost lease, node stopped mid-turn, a commit that did not get through) ends its span with
  `cicada.outcome` = `abandoned` and no `exception` event, also when its handler failed.
- **One trace across actors.** The span's parent is the trace context in the message's headers (W3C `traceparent`).
  Every message a turn sends carries the turn's trace context automatically, so the turns of a chain form one trace
  with no code in the handlers. A send that sets `traceparent` itself keeps its own. From outside an actor, use
  `current_trace_headers()`; over HTTP, put `traceparent` in the message's `headers`. `with_propagator(..)` replaces
  the global propagator.
- **Metrics:** `cicada.turn.duration` (claim to commit) and `cicada.message.lag` (due time to claim), histograms in
  seconds by `cicada.actor.type` and `cicada.message.type`; the duration also by `cicada.outcome`.

## Testing with `TestActors`

`cicada-test` runs types against an in-memory store whose clock is tokio's paused time, so delays, timers,
heartbeats and turn timeouts are deterministic and instant. Run each test with `#[tokio::test(start_paused = true)]`
(tokio's `test-util` feature). The blocks of this section test the types of
[examples/src/sync.rs](../../examples/src/sync.rs):

<!-- doctest
@@17
# #[tokio::main(flavor = "current_thread", start_paused = true)]
# async fn main() {
@@16
# }
-->
```rust
use std::time::Duration;

use cicada_examples::sync::{Page, Reports, ReportsActor, ReportsState, Sync, SyncActor, SyncApi, SyncMsg, SyncState};
use cicada_test::TestActors;

struct FakeApi; // three pages: 10, 10 and 5 items

impl SyncApi for FakeApi {
    async fn fetch(&self, _source: &str, cursor: Option<&str>, _key: &str) -> Page {
        match cursor {
            None => Page { size: 10, next: Some("p2".into()) },
            Some("p2") => Page { size: 10, next: Some("p3".into()) },
            Some(_) => Page { size: 5, next: None },
        }
    }
}

let test = TestActors::builder()
    .with(|node| node.host(SyncActor { api: FakeApi }).host(ReportsActor))
    .build()
    .await
    .unwrap();
let (acme, reports) = (Sync { tenant: "acme".into() }, Reports { tenant: "acme".into() });
test.create(&reports, &ReportsState::default()).await; // a send to an absent instance fails the turn
test.create(&acme, &SyncState { source: "https://api.acme.test/items".into(), imported: 0 }).await;
test.send(&acme, SyncMsg::Run).await;
test.run_until_idle().await; // every due message handled
let next = test.pending(&acme).await; // the delayed Run
assert_eq!(cicada::time::between(test.now(), next[0].deliver_at), Duration::from_secs(3600));

test.advance_time_by(Duration::from_secs(2 * 3600)).await; // handles every message at its due time on the way
assert_eq!(test.state::<_, ReportsState>(&reports).await.runs, vec![25, 25, 25]);
assert!(test.failures().await.is_empty());
```

`TestActors` also offers `send_with`, `send_raw` (the raw result, refusals included),
`run_until_idle_max`, `pending_at`, `list(prefix)`, `exists`, `state_json`, `version`, `outcome(&message)`, `now`,
`clock`, `system()` for anything else (every `ActorSystem` call) and `store()`, the in-memory `Store`, to build a
second system on it. The builder's `with(|node| …)` configures the node like `ActorSystem::builder` (hosted types,
observers, `default_max_turn`, …); `start_at` and `retention` set the store's start time and retention. A test that
plays a node which claims a turn and dies calls `claim_and_abandon(test.store(), &types, lease, limit)`: the message
is claimed again, as its next attempt, once the lease runs out. That claim is lost, so a message abandoned as often as
the [claim limit](#claim-limit) allows ends `failed`. A test without `TestActors` opens the in-memory store
with `Store::memory()`, `Store::memory_on(clock)` or `Store::memory_with(clock, retention)`. Nothing runs in the background: turns run only inside
`run_until_idle` and `advance_time_by`.

A dependency that needs a clock or a timer of its own (a rate limiter, a cache with expiry) uses `tokio::time`, which
is the test's virtual time; `test.clock()` is the store's virtual wall clock.

### Contract snapshots

A node refuses to start when a type's contract differs from the one registered for its type name, which in a rolling
deploy shows up in production. `assert_contracts_match` moves that failure into a test: it keeps each type's contract
as a JSON file and fails, naming what changed (state schema, message types added, removed or changed), when the code
no longer matches. In a test:

<!-- doctest
@@2
# use cicada_examples::sync::{Page, ReportsActor, SyncActor, SyncApi};
# struct FakeApi;
# impl SyncApi for FakeApi {
#     async fn fetch(&self, _: &str, _: Option<&str>, _: &str) -> Page { Page { size: 0, next: None } }
# }
@@5
-->
```rust,no_run
use cicada::actor::contract_of;
use cicada_test::assert_contracts_match;

assert_contracts_match("tests/contracts", &[
    contract_of::<SyncActor<FakeApi>>().unwrap(), // fails with the type's problems when it is invalid
    contract_of::<ReportsActor>().unwrap(),
]);
```

A missing file is written (commit it). To accept a change, delete the file or run the tests with
`CICADA_UPDATE_CONTRACTS=true`. The files are named after the type (`sync` →
`sync.json`) and hold the contract as `GET /types` serves it.

### State samples

State that no longer decodes fails every turn of its instance. Keep samples of the shapes a state ever had and check
them with the runtime's own decoder (serde: unknown keys are ignored, missing fields need `#[serde(default)]`):

<!-- doctest
@@1
# use cicada_examples::sync::SyncState;
@@5
-->
```rust,no_run
use cicada_test::{assert_state_decodes, record_state_sample};

let dir = "tests/states/sync";
let first_run = SyncState { source: "https://api.acme.test/items".into(), imported: 0 };
record_state_sample(dir, "first-run", &first_run); // written once, never overwritten
assert_state_decodes::<SyncState>(dir); // every sample in the directory, also those of earlier versions
```

Decoding is all it checks: a sample whose renamed field is now ignored still decodes, with the default in its place.

## Examples

[`examples/`](../../examples/) (crate `cicada-examples`) holds fifteen worked examples, each with its tests under
`examples/tests/`, run with `TestActors` on virtual time:

| Module | Pattern |
| --- | --- |
| `singleton` | A singleton that keeps no data (`maintenance.cleanup/_`, `NoState`) whose turns never overlap. |
| `timer` | A timer / timeout: a delayed self-send, cancelled by its id. |
| `cron` | A cron job; each tick does the work and arms the next one in the same commit. |
| `reply` | Request and reply; the responder replies to `message().from` with the correlation id. |
| `saga` | An orchestrator saga with compensation, and per-message retry set by the sender. |
| `steps` | Durable steps; every step is its own turn, so a committed step never runs again. |
| `fanout` | Fan-out and fan-in; one child per item, the parent counts the replies. |
| `registry` | A registry actor that keeps the refs of the instances it created. |
| `supervision` | A supervisor restarts a mailer that fails; dead letters keep the failed mail, to redrive it. |
| `rate_limit` | A shared rate limit; signups create a limiter and submit their texts to it in one turn. |
| `debounce` | A burst of edits, one reindex; each edit submits the latest version to a debouncer. |
| `batching` | Audit events in bulk; each change adds its event to a batcher that writes 50 at once. |
| `flow_onboarding` | The onboarding of `steps` as a durable flow: async code, one new step per turn, and a durable sleep. |
| `flow_events` | Flows started by an event: a placed order starts a receipt and a fulfilment in one commit, a batcher a digest. |
| `sync` | The architecture's example: a paged import, one turn per page, rescheduled hourly. |

Every example has the same shape: a public ref struct per actor type (its one field is the key), a
message enum with `#[serde(tag = "type")]`, a state struct, and an actor struct whose fields are its dependencies
(external services, faked in the tests), with one `impl Actor` whose `handle` matches on the message. The flow
examples write a `Flow` instead, which `FlowActor` hosts ([Flows](actors.md#flows)).
