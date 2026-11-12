# Cicada for Rust — durable actors

Cicada is a small durable actor system for Rust services. An actor is a row of state plus a mailbox in a shared
transactional store (PostgreSQL or SQLite); it occupies memory only while one of its messages is being handled. Every
service that runs an `ActorSystem` is a *node*. Nodes are peers that coordinate only through the store: there is no
coordinator and no cluster protocol.

- **Durable turns.** Claim → load → handle → commit. New state, outgoing messages, creates and the message's outcome
  are written in one transaction; a node that dies mid-turn loses its lease and the turn is re-run elsewhere.
- **One turn at a time per actor**, messages in delivery-time order, delayed messages never early, exactly one
  outcome per message (`done`, `failed`, `cancelled`, `purged`).
- **Typed and checked.** Actor addresses are typed refs with a type name and a key (`sync/acme`); the compiler
  checks that a ref accepts a message, and that a handler covers every message of its family.
- **One HTTP API** on every node as an axum `Router`, described by an OpenAPI document, a typed client, an in-process API,
  `TestActors` on tokio's paused time, and per-message retry policies set by the sender.

The design — domain language, guarantees G1–G6, store requirements S1–S10, the HTTP contract — is in
[docs/architecture.md](docs/architecture.md); the decisions in [docs/adr/](docs/adr/); how to size,
deploy, upgrade and watch nodes in [docs/operations.md](docs/operations.md). The most important decisions are
summarised [below](#decisions).

## Crates

| Crate | Contents |
| --- | --- |
| `cicada` | Addressing (`ActorAddress`, `ActorRef`, `TypeName`), the definition API (the `Actor` trait), the runtime and dispatch loop (`ActorSystem`) with per-message retry (`RetryPolicy`) and retention (`Retention`) and the claim limit (`NodeConfig::max_lost_claims`), watches (`Turn::watch`, `Failed`, `Stopped`), the store (`Store`), turn observers (`TurnObserver`), contracts and JSON Schemas, the HTTP wire model (`cicada::api`). No HTTP server. Each store is a cargo feature, none on by default: `sqlite` (`Store::sqlite`: one file, WAL, one writer; for a single node or a few processes on one host) and `postgres` (`Store::postgres`: `FOR UPDATE SKIP LOCKED` claims, `LISTEN/NOTIFY` wake-ups; for many nodes). The feature `testing` is the test support that `cicada-test` exports. |
| `cicada-axum` | `actor_api()` (the whole API), `actor_api_read()` (only what reads) and `actor_health()` (a public readiness check) as axum routers; the check of a send and a create against the registered JSON Schemas, and the sample checks `assert_message_sample` and `assert_state_sample`. |
| `cicada-client` | `ActorClient`: typed HTTP client (reqwest) for remote callers, idempotent sends. |
| `cicada-cli` | The binary `cicada`: kubectl-style commands and a k9s-style terminal UI over the HTTP API; see [Command line and terminal UI](#command-line-and-terminal-ui). |
| `cicada-test` | `TestActors` on the in-memory store (`Store::memory`), virtual time, `claim_and_abandon` (a node that dies), contract snapshots and state samples. |
| `cicada-extras` | Building blocks on the public API, no core changes: dead letters (`DeadLettersActor`, `FailureWatcher`), `Schedule`. |
| `cicada-otel` | `ActorTelemetry`: OpenTelemetry traces and metrics per turn, an observer on the OpenTelemetry API. |
| `examples` (`cicada-examples`) | Ten worked examples, each tested with `TestActors`; see [Examples](#examples). |
| `sample-service` | A runnable node hosting the examples, configured by environment variables. |

## Quick start

### Define an actor type

A ref struct names the type, holds the key, one name, and names the messages: an enum with `#[serde(tag = "type")]`
(the variant name is the type name on the wire). The actor's one `handle` takes that enum and matches on it, so the
compiler checks that every message is handled. A message that cannot be decoded when it is delivered (a type name the
enum does not have, or a payload that does not fit) ends `failed` with the reason, without a retry, and no handler runs.
This is [examples/src/sync.rs](examples/src/sync.rs), shortened:

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
`String` as its serde form, as `BatchItem` in [examples/src/fanout.rs](examples/src/fanout.rs) does:

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
the send limits with `TurnError::Invalid`, which the handler may handle.

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

### Child actors

An actor may have a **parent** (ADR 0019). `turn.create_child(&ref, &state)` creates an actor whose parent is the
actor that runs the turn; `turn.create` and every create from outside a turn (`ActorSystem::create`, `PUT`) create an
independent one. The parent is stored with the child and never changes, and the address stays `type/key`. Deleting an
actor (`delete_actor`, `DELETE /actors/{type}/{key}`, `turn.delete`) or stopping it (`turn.stop()`) removes it and
every descendant (its children, theirs, and so on) in one transaction: their pending messages end `purged`, and a turn
running on one commits nothing. Pause, resume and purge apply to one actor only.

A child create of a child of the same parent that exists already leaves it untouched, as every create does. It never
takes over an actor: one that exists as an independent actor or as a child of another one makes the commit fail, and
the turn's message ends `failed`, as for a send to an absent actor. A turn that creates one address both ways gets
`TurnError::Invalid` at the second call. The `fanout` example creates its items as children, so deleting a batch
removes the items that have not reported yet.

```rust
# use cicada::prelude::*;
# use cicada_examples::fanout::{BatchActor, BatchItem, BatchItemState};
# fn start(turn: &mut Turn<BatchActor>) -> HandlerResult {
let item = BatchItem { batch: turn.self_ref().id.clone(), index: 0 };
turn.create_child(&item, &BatchItemState { input: "x".into() })?; // removed with this batch
# Ok(())
# }
```

`ActorSystem::get_actor` and `GET /actors/{type}/{key}` name the parent (`parent`, null for an independent actor);
`ActorSystem::list_children`, `ActorClient::children`, `GET /actors/{type}/{key}/children` and `cicada get children`
list the direct children in address order, one level at a time. A large tree is one large transaction; see the
operations guide, On call.

### Watches

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
contract (see the operations guide, Changing a contract).

The store writes a notification in the transaction of its event, one per watcher and actor, and marks it
(`notification` in the views of a message):

- A **`Failed`** when a message ends `failed`, for the watchers of the actor's address, of a prefix of its type and of
  `*`. It holds the actor, the failed message's id, type, payload, correlation id and sender (`from`), the error and the
  attempts, and is kept as long as the failure is: by the `failed` rule of the failed message's retention. A message
  that is retried, done or cancelled tells nobody.
- A **`Stopped`** when an actor watched by its address stops itself, is deleted, or goes with its parent: `cause` is
  `stopped`, `deleted` or `parent`, and `by` the actor whose turn removed it, `null` for a delete from outside. A watch
  on a type prefix or on `*` tells of failures only.

A notification that fails tells nobody, so two actors that watch each other cannot loop; one sent again from outside
(`retry`) is an ordinary message. A watcher that does not exist, or that the same event removes, gets nothing, and the
watched actor never knows its watchers. From outside a turn, `ActorSystem::watch`, `unwatch` and `watches`, `PUT`,
`DELETE` and `GET /watches`, `ActorClient::watch`, and `cicada create watch` set, remove and list watches; the watcher
must exist, and its type must declare both variants.

### Control effects

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
stays its child:

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

The node logs each control effect of a turn it committed at the info level (see the operations guide, Logs).

### Run it in an axum service

The host owns the store and the node's life cycle; the HTTP API is a `Router` it mounts with its own
authentication:

<!-- doctest
@@11
# use cicada_examples::sync::{Page, ReportsActor, Sync, SyncActor, SyncApi, SyncMsg};
# struct HttpSyncApi;
# impl SyncApi for HttpSyncApi {
#     async fn fetch(&self, _: &str, _: Option<&str>, _: &str) -> Page { Page { size: 0, next: None } }
# }
@@58
-->
```rust,no_run
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use cicada::store::{PostgresOptions, SqliteOptions};
use cicada::{ActorSystem, SendResult, Store};
use cicada_axum::{actor_api, actor_health};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = match std::env::var("DATABASE_URL") {
        Ok(url) => Store::postgres(&url, PostgresOptions::default()).await?, // feature "postgres"
        Err(_) => Store::sqlite("actors.db", SqliteOptions::default()).await?, // feature "sqlite"
    };
    let system = ActorSystem::builder(store.clone())
        .host(SyncActor { api: HttpSyncApi }) // this node runs turns for these types
        .host(ReportsActor)
        .build();
    system.start().await?; // validates the types and registers their contracts

    let token: Arc<str> = std::env::var("API_TOKEN")?.into();
    let own_routes = Router::new()
        .route("/tenants/{tenant}/sync", post(start_sync)) // your own routes use the in-process API
        .with_state(system.clone());
    let app = Router::new()
        .merge(actor_health(system.clone(), "/ready")) // GET /ready: public readiness, for probes
        .merge(actor_api(system.clone()).layer(middleware::from_fn_with_state(token, bearer))) // the whole API
        .merge(own_routes);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            system.stop().await; // no new claims; running turns may finish within the grace period
        })
        .await?;
    store.close().await; // the store is yours: close it after the node stopped
    Ok(())
}

async fn start_sync(State(system): State<ActorSystem>, Path(tenant): Path<String>) -> StatusCode {
    match system.send(&Sync { tenant }, SyncMsg::Run).await {
        Ok(SendResult::Accepted(_)) => StatusCode::ACCEPTED,
        Ok(SendResult::NoSuchInstance) => StatusCode::NOT_FOUND,
        Ok(_) | Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// The host's authentication: the API has none of its own.
async fn bearer(State(token): State<Arc<str>>, req: Request, next: Next) -> Response {
    let expected = format!("Bearer {token}");
    match req.headers().get(header::AUTHORIZATION) {
        Some(value) if value.as_bytes() == expected.as_bytes() => next.run(req).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("a SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
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
  set `sslmode` on purpose ([docs/operations.md, TLS for PostgreSQL](docs/operations.md#tls-for-postgresql)).
  `Store::sqlite(path, options)` sets up a new or empty file and refuses a SQLite file that holds the tables of
  another program.
- A node hosting no types still serves the whole API (create, send, query) for every registered type.
- `actor_api(system)` and `actor_health(system, path)` are plain routers with no fallback: `merge` them or `nest`
  them under a prefix, and wrap them in your own layers. They need nothing installed on the app: they read and write
  JSON themselves with the wire types of `cicada::api`. `actor_api_with(system, ApiConfig { max_request_body_bytes })`
  changes the body limit.
- `ActorSystem` is cheap to clone; every clone is the same node. Its methods are the in-process API: `create`,
  `send`, `send_with`, `send_to`, `send_message`, `get_actor`, `delete_actor`, `list_actors`, `pending`,
  `list_pending`, `status`, `outcome`, `cancel`, `failures`, `types`, `health`.
- The runtime logs with [`tracing`](https://docs.rs/tracing); install a subscriber (such as `tracing_subscriber`) to
  see its output.

Create and start an instance from outside (the report target first: a send to an absent instance fails the turn):

```sh
curl -X PUT  localhost:8080/actors/reports/acme -H "Authorization: Bearer $API_TOKEN" -d '{"state":{}}'
curl -X PUT  localhost:8080/actors/sync/acme -H "Authorization: Bearer $API_TOKEN" \
     -d '{"state":{"source":"https://api.acme.test/items"}}'
curl -X POST localhost:8080/actors/sync/acme/messages -H "Authorization: Bearer $API_TOKEN" -d '{"type":"Run"}'
```

or with the typed client (`cicada-client`):

<!-- doctest
@@1
# use cicada_examples::sync::{Reports, ReportsState, Sync, SyncMsg, SyncState};
# async fn example() -> Result<(), cicada_client::ClientError> {
@@8
# Ok(()) }
-->
```rust,no_run
use cicada_client::ActorClient;

let client = ActorClient::new("http://localhost:8080"); // or ActorClient::with_client(url, reqwest_client)
client.create(&Reports { tenant: "acme".into() }, &ReportsState::default()).await?;
let state = SyncState { source: "https://api.acme.test/items".into(), imported: 0 };
client.create(&Sync { tenant: "acme".into() }, &state).await?;
let receipt = client.send(&Sync { tenant: "acme".into() }, SyncMsg::Run).await?; // client-generated id, retried with it
let status = client.status(&receipt.id).await?; // Pending, Running, Done, Failed, Cancelled, Purged
println!("{:?}", status.map(|s| s.status));
```

The client takes the same ref and message types as the actor definitions. Authentication is the `reqwest::Client`'s:
give `ActorClient::with_client` one with default headers. Expected alternatives are values (a missing instance is
`None`, a duplicate send is `receipt.duplicate`, a cancel returns a `CancelOutcome`); every other error response is
`ClientError::Api` with the server's problem. A send that fails on I/O or with `503` is retried with the same id (3
attempts by default), so it is enqueued at most once.

The Rust snippets in this README are doctests of the `examples` crate (`cargo test -p cicada-examples --doc`), so
they keep compiling, and the ones that need no server or database run.

## HTTP API

`{type}` is the type name and `{key}` the key of the address (`sync/acme`), the key percent-encoded as in the address. Errors are `application/problem+json` (RFC 9457) with `type` `urn:cicada:problem:<slug>` and the
request's path and query as `instance`.

| Endpoint | Success | Errors |
| --- | --- | --- |
| `PUT /actors/{type}/{key}` body `{state}` — create; `state` is required (`{}` for `NoState`) | 201 created; 200 already exists (untouched) | 400 no body or no `state`; 404 no such type; 422 `invalid-state`: a `null` state or one that does not match the registered state schema (every node), or one that cannot be decoded (hosting node); 422 `invalid-address`: a name the ref does not accept (hosting node) |
| `GET /actors/{type}/{key}` — address, type, state, version, createdAt, paused, parent (null for an independent actor) | 200 | 404 |
| `DELETE /actors/{type}/{key}` — admin delete of the actor and every descendant, purges their mailboxes | 204 | 404 |
| `GET /actors/{type}/{key}/children?cursor=&limit=` — the direct children's addresses, in address order, paged | 200 | 404 |
| `GET /actors?prefix=&cursor=&limit=` — instance addresses, paged | 200 | |
| `POST /actors/{type}/{key}/messages` body `{id?, type, payload?, deliverAt? \| delay?, correlationId?, headers?, retry?, retention?}` — send | 202 `{id, deliverAt}`; 200 that id was already accepted (same target and type) | 400 invalid `retry` or `retention` policy; 404 `no-such-type` when no registered type matches the target, also when the actor exists; 404 `no-such-instance` when its type is registered and the actor does not exist; 409 id already used for another target or type; 422 unknown message type; 422 `invalid-payload`: a payload that does not match the registered schema of its message type (every node), or one that cannot be decoded (hosting node) |
| `GET /actors/{type}/{key}/mailbox?cursor=&limit=` — pending mailbox in delivery order, each with `attempt`, `failures`, `lostClaims`, `lastError`, `retry` and `state` (`running` or `pending`) | 200 | 404 |
| `POST /actors/{type}/{key}/mailbox/pause` — pause: messages queue up, no new turn is started, a running turn finishes | 204 | 404 |
| `POST /actors/{type}/{key}/mailbox/resume` — resume a paused actor | 204 | 404 |
| `POST /actors/{type}/{key}/mailbox/purge` — every pending message except one being handled ends `purged`; the actor stays | 200 `{purged}` | 404 |
| `GET /actors/{type}/{key}/turn` — the running turn: message, node, lease expiry, `cancelRequested` | 200 | 404 `no-running-turn` |
| `POST /actors/{type}/{key}/turn/cancel` — ask the owning node to cancel the running turn, within one heartbeat; the message ends `cancelled`, without effects | 202 | 404 `no-running-turn` |
| `GET /messages?prefix=&cursor=&limit=` — pending messages of every instance under the prefix (including running ones), instance by instance in address order, each in delivery order | 200 | |
| `GET /messages/{id}` — status, error, `failures` and `lastError`, node and lease while running, envelope (with `lostClaims`, `retry`, `retention`, `causedBy` and `notification`, true for a notification of a watch) | 200 | 404 unknown, past retention, or removed by its retention policy |
| `POST /messages/{id}/cancel` — cancel a pending message (a running turn is stopped with `turn/cancel`) | 204 | 404; 409 running or already finished |
| `POST /messages/{id}/retry` — send a `failed` or `cancelled` message again: a new message with a new id, same target, type, payload, correlation id, headers, retry and retention, `causedBy` the original | 202 `{id, deliverAt}` | 404 unknown, past retention or removed by its retention policy; 404 `no-such-instance` when the target is gone; 409 `message-not-retryable` for a pending, running, done or purged message |
| `POST /messages/{id}/deliver-now` — make a pending, not running message due at once, at a new place in delivery order | 200 message | 404; 409 running or already finished |
| `POST /messages/bulk-cancel` body `{ids? or prefix?, messageType?}` — cancel by ids (at most 1000) or by filter, at most 1000 per call; skips what cannot be cancelled | 200 `{affected, more}` | 400 not exactly one of ids or a filter, or more than 1000 ids |
| `POST /messages/bulk-retry` body `{ids? or prefix?, messageType?}` — retry by ids or by filter (over `failed` outcomes), at most 1000 per call; a filter skips messages already retried | 200 `{affected, more}` | 400 |
| `GET /messages/{id}/caused?cursor=&limit=` — the messages that the turn which handled `{id}` sent, one level down: pending ones and retained outcomes, each like `GET /messages/{id}`, in message-id order | 200 | 404 unknown or past retention |
| `GET /outcomes?status=&prefix=&since=&cursor=&limit=` — finished messages with envelope and error; `status` is `done`, `failed`, `cancelled` or `purged` (absent: all), oldest first | 200 | 400 unknown status |
| `GET /turns?node=&prefix=&cursor=&limit=` — the turns running now, by address: actor, message, node, lease expiry, `cancelRequested` | 200 | |
| `GET /nodes` — every node: id, hosted types, started and last seen, status (`alive`, `suspect`, `dead`), `runningTurns` (count) | 200 | |
| `GET /nodes/{node}` — one node, and the list of its running `turns` | 200 | 404 `no-such-node` |
| `GET /stats?prefix=` — `{actors, pending, oldestDue}`: the number of actors and pending messages under the prefix, and the delivery time of the oldest message that is due (null when none is). It reads every row under the prefix | 200 | |
| `GET /types` — type names, contract hashes, state and message JSON Schemas | 200 | |
| `GET /types/{name}` — one type | 200 | 404 |
| `DELETE /types/{name}` — removes the type's registration (a node that hosts the type registers it again at its next start) | 204 | 404; 409 actors of the type exist |
| `GET /watches?watcher=&target=&cursor=&limit=` — stored watches `{target, watcher}` in the order of target and watcher; `target` narrows to the watches stored with exactly that target | 200 | 400 a watcher or target that does not parse |
| `PUT /watches` body `{target, watcher}` — set a watch; the target is an address, a type prefix or `*`, and need not exist | 201 set; 200 set already | 400 a target that is no target; 404 `no-such-type` or `no-such-instance` for the watcher; 422 `not-a-watcher` when its type does not declare `Failed` and `Stopped` |
| `DELETE /watches?target=&watcher=` — remove a watch | 204 | 400; 404 `no-such-watch` |
| `GET /openapi.json` — the OpenAPI 3.1 document of this router | 200 | |
| `GET /health` — node running and store reachable | 200 | 503 |

`actor_api(system)` serves all of this. `actor_api_read(system)` serves only what reads: it has no route that changes
anything, so it can be exposed more widely. Authentication stays the host's for both.

Clients can be generated from the OpenAPI document. A copy is in [docs/openapi.json](docs/openapi.json), and a test fails
when it no longer matches the routes (`CICADA_UPDATE_OPENAPI=true` updates it). Payloads and states are free-form JSON in
the document; the schemas of a type come from `GET /types/{name}`. The document is OpenAPI 3.1, which not every generator
reads yet (Progenitor reads 3.0 only), so `cicada-client` stays hand-written.

On every endpoint, additionally:

- `400` — malformed JSON, path or query; an unknown key in a request envelope (`{"paylaod": …}`, also inside `retry`
  and `retention`); both `deliverAt` and `delay`; a delivery time beyond year 9999; envelope limits (id/type ≤ 200
  chars, ≤ 64 headers, no NUL); an invalid retry policy; `limit` outside 1..1000; a bad cursor. Payloads and states
  themselves are checked against their schemas instead (see [The schema check](#the-schema-check)).
- `405` — `POST /actors/{type}/{key}` without `/messages`.
- `413 request-too-large` — a body larger than `ApiConfig::max_request_body_bytes` (default 1 MiB), by its
  `Content-Length` or while reading.
- `503 store-unavailable` — the store cannot be reached (worth retrying; the client retries sends with the same id).
- `500 internal-error` — anything unexpected, such as stored data that no longer decodes, or a panic (not worth
  retrying as such).
- `401`/`403` — whatever the host's authentication layer answers.

`delay` is an ISO-8601 duration (`"PT5S"`, `"PT1H30M"`; a negative one is `400`); `deliverAt` is an ISO-8601
instant. Both are resolved against the store clock. Listings take `cursor` (the previous page's `next`) and `limit`
(default 100); a `prefix` is a prefix of the address (`sync/`, `sync/ac`). Pages are a keyset,
not a snapshot: in a mailbox listing, a message rescheduled in place between two pages (later `deliverAt`, new `seq`)
can appear on both, so deduplicate by id.

### The schema check

A send and a create over HTTP are checked against the registered JSON Schema of their type, on every node, whether or
not the node hosts the type (ADR 0026): the payload against the schema of its message type, the state of a create
against the state schema. One that does not match is refused with `422 invalid-payload` or `422 invalid-state`, and
nothing is stored. A node that hosts the type then decodes it with serde as well, with the same answer. The detail
says which check refused it, and where: `Payload of 'Add' does not match the schema: at /n: "x" is not of type
"integer"` names the first place that does not match (a value that may have one of several shapes, such as an
`Option` of a struct, is named as a whole, with a general reason); `Payload of 'Add' cannot be decoded: …` is serde's.
Either is cut at 2,000 characters with the mark ` … (cut)`. The check asks only for that first place, so a refused
request costs no more memory for many mismatches than for one.

The order of the checks stays: a send that repeats the id of a message that is pending, or whose outcome is still
shown, is answered as a repeat (`200`) before it is checked; an unknown message type is `422 unknown-message-type`.
In a create the schema comes before what only a hosting node checks, so a state that does not match gets the same
answer on every node; a name that the ref does not accept (`422 invalid-address`) is judged after it, by a hosting
node.

What the door does not check:

- **Formats.** `format` (`date-time`, `email`, …) is a note, as draft 2020-12 has it: a text that is no date passes the
  door, and the message fails in its turn.
- **A schema the validator cannot use.** A schema that does not compile (a `$ref` outside the document, a pattern its
  regular expressions do not know) is not enforced; a warning names it once. The check loads nothing from outside the
  schema: no file, no network.
- **JSON that code sends.** `turn.send_raw` and `ActorSystem::send_message` are not checked; only a hosting node's
  decoder judges them, as before. Typed sends are checked by the compiler.

The schema and serde come from the same type, but can disagree. A payload that serde accepts and the schema refuses is
refused at the door: a field sent under its `#[serde(alias)]`, a form that only a hand-written `Deserialize` takes,
a payload that repeats `"type"` for a family that denies unknown fields (the decoder drops it, the schema does not
allow it), or what the type itself writes when a field has `#[serde(with)]` and no `#[schemars(with)]`. In the last
case every send of the typed client is refused, there is no switch, and the way out is a right schema, a contract
change (see [Changing a contract](docs/operations.md#changing-a-contract)). The other way round, a payload that only
serde refuses passes a node that does not host the type and fails in its turn. Test what clients send, and what the
types write, with the sample checks of `cicada-axum`; each fails, naming which of the two refused the sample and why:

<!-- doctest
@@2
# use cicada_examples::sync::{SyncMsg, SyncState};
@@4
-->
```rust
use cicada_axum::{assert_message_sample, assert_state_sample};
use serde_json::json;

assert_message_sample::<SyncMsg>(&serde_json::to_value(SyncMsg::FetchPage { cursor: None }).unwrap()); // what it writes
assert_message_sample::<SyncMsg>(&json!({"type": "FetchPage", "cursor": "p2"})); // what a client sends
assert_state_sample::<SyncState>(&json!({"source": "https://api.acme.test/items", "imported": 3}));
```

## Command line and terminal UI

`cicada-cli` builds the binary `cicada`. It talks to a node only over HTTP, through `cicada-client`, so it works
against any node that serves `actor_api()`; the commands that only read also work against `actor_api_read()`. Build it
from a checkout with `cargo install --path crates/cicada-cli`, or run it with `cargo run -p cicada-cli -- <command>`.

**Which server.** The server is `--server`, else `CICADA_SERVER`, else the `server` of the context. The context is
`--context`, else `CICADA_CONTEXT`, else the file's `current-context`; a context the file does not have is an error.
The token is `--token`, else `CICADA_TOKEN`, else the context's `token`, sent as `Authorization: Bearer <token>`; the
context's `headers` go with every request. The file is `$CICADA_CONFIG`, else `$XDG_CONFIG_HOME/cicada/config.toml`,
else `~/.config/cicada/config.toml` (on macOS too); a missing file counts as an empty one.

```toml
current-context = "local"

[contexts.local]
server = "http://localhost:8080"

[contexts.prod]
server = "https://cicada.internal/api"
token = "…"
headers = { "X-Tenant" = "ops" }
```

`cicada config set-context NAME --server URL [--token T] [--header K=V]` adds or replaces a context (the first one also
becomes current), `config use-context NAME` switches, `config get-contexts` and `config current-context` show them.
They rewrite the whole file, so comments in it are lost. On Unix the file has mode 0600 after every write, because it
holds tokens.

**Commands.** Verb first, as in kubectl:

```sh
cicada get actors --prefix account/                                  # one page of --limit (100); --all reads every page
cicada get actor account/acme:7 -o json                              # the API's view; -o yaml, -o wide, or a table
cicada get children batches/b1                                       # the direct children, one row each
cicada send account/acme:7 --type Deposit --payload '{"amount":5}'   # prints the message id
cicada get outcomes --status failed --since 2026-11-10T00:00:00Z
cicada retry messages --prefix account/ --type Deposit               # bulk: --id ID (repeatable) or a filter
```

| Command | What |
| --- | --- |
| `get actors\|messages\|outcomes\|turns [--prefix P]`, `get nodes\|types` | Listings; also `outcomes --status S --since T` and `turns --node N` |
| `get actor\|children\|mailbox\|turn ADDR`, `get message\|caused ID`, `get node ID`, `get type NAME` | One item (`-o wide` shows an actor's `PARENT`); an actor's direct children; a mailbox; the running turn; what a message caused |
| `describe actor\|message\|node\|type X` | One item in full, as YAML unless `-o json` |
| `create actor ADDR --state JSON`, `delete actor ADDR`, `delete type NAME` | Create (`--state '{}'` for `NoState`) or delete an actor with its descendants; remove a type's registration |
| `send ADDR --type T [--payload JSON] [--id ID] [--delay PT5S \| --at INSTANT] [--correlation-id C] [--header K=V]` | Send a message |
| `cancel message ID`, `cancel turn ADDR`, `retry message ID`, `deliver-now ID` | One message; stop a running turn |
| `cancel messages`, `retry messages`, with `--id ID…` or `--prefix P [--type T]` | Bulk, at most 1000 per call |
| `pause ADDR`, `resume ADDR`, `purge ADDR` | An actor's mailbox |
| `get watches [--watcher ADDR] [--target T]`, `create watch --target T --watcher ADDR`, `delete watch --target T --watcher ADDR` | List, set and remove watches; `-o wide` on a message shows `NOTIFICATION` |
| `stats [--prefix P]`, `health` | Counts; whether the node and its store answer |

A missing item (`actor account/nobody:1 not found`) or an error response (the problem's title and detail) prints
`error: …` and exits with code 1.

**Terminal UI.** `cicada tui` (with the same flags, e.g. `cicada --server http://localhost:8080 --token secret tui`
against the sample service with `API_TOKEN=secret`) opens on the actors. The header shows the context, the server, the
nodes by health, the number of actors, pending messages and running turns, how long the oldest due message has waited,
and the keys of the screen. The screen refreshes every 2 s and the header every 10 s (`GET /stats` reads every row).
Requests run beside the screen, so a slow node never freezes it. Status words are coloured; failed rows are red.

| Keys | What |
| --- | --- |
| `:actors`, `:messages`, `:outcomes [status]`, `:turns`, `:watches`, each with an optional prefix; `:nodes`, `:types` | A listing; Tab completes; Enter on a watch opens its watcher |
| `:actor ADDR`, `:mailbox ADDR`, `:message ID`, `:node ID`, `:type NAME` | One item |
| `enter`, `esc`, `q` | Open the selected row; clear the filter or go back; `q` on the first screen and `ctrl-c` quit |
| `j` `k` `↑` `↓`, `g` `G`, `pgup` `pgdn` | Move |
| `/`, `<` `>`, `s`, `w` | Filter rows as you type; sort column; sort order; wide columns |
| `tab` | The next tab of an overview (an actor's mailbox, outcomes and children) |
| `d` | Describe: the item as YAML; `t` switches to JSON, `w` wraps |
| `m`, `a`, `b`, `P` | An actor's mailbox; a message's actor; its cause; an actor's parent |
| `c`, `r`, `n` | Cancel, retry, deliver now a message |
| `p`, `u`, `x` | Pause, resume, purge an actor's mailbox |
| `ctrl-k`, `ctrl-d` | Stop the running turn; delete an actor or remove a type |
| `ctrl-r`, `?` | Refresh now; help |

The screen of an actor shows its parent (or none) and lists its direct children in the tab `children`: Enter on a child
opens the child's screen, `P` the parent's and Esc goes back, so the tree is walked one level per screen. An action
applies to the selected row, else to the item the screen shows, and runs only after a confirmation whose default is No
(`y` confirms); the question before a delete says that the descendants go too. The mouse wheel moves, a click selects a
row, a click on the selected row opens it and a click on a column header sorts by it. Sending messages and bulk actions
are CLI only.

## Testing with `TestActors`

`cicada-test` runs types against an in-memory store whose clock is tokio's paused time, so delays, timers,
heartbeats and turn timeouts are deterministic and instant. Run each test with `#[tokio::test(start_paused = true)]`
(tokio's `test-util` feature):

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
[docs/operations.md](docs/operations.md) says what counts in practice and how to choose a limit.

## Message persistence

How long a finished message stays readable is also part of its delivery, chosen by the sender next to `retry`: a
`Retention` policy with one rule for a message that ends `done` and one for a message that ends `failed`. A missing
rule, and the outcomes `cancelled` and `purged`, keep the store default (the store's retention, 7 days by default).

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

## Extras

`cicada-extras` holds building blocks that use only the public API (observers and the operations of `ActorSystem`).

**Dead letters.** A failed message stays in the failures list for the outcome retention. `FailureWatcher` (a job, not
an actor: handlers cannot read `/outcomes`) copies each failure into the `DeadLetters` singleton
(`cicada.dead-letters/_`, hosted as `DeadLettersActor`), which keeps them past the retention, at most `capacity`
(oldest dropped first):

<!-- doctest
# use std::sync::Arc;
# use std::time::Duration;
@@2
# async fn node(store: cicada::Store) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
@@5
# Ok(())
# }
-->
```rust
use cicada::ActorSystem;
use cicada_extras::{DeadLettersActor, FailureWatcher};

let system = ActorSystem::builder(store).host(DeadLettersActor::with_capacity(1_000)? /* , … */).build();
system.start().await?;
let watcher = FailureWatcher::new(system.clone()).prefix("orders/").start(Duration::from_secs(10));
// … on shutdown: watcher.abort();
```

The watcher works through an `ActorSystem`; a process that hosts no types can run one on a send-only node. Every node
may run one: each failure is forwarded with an id derived from the failed message's id, so the store's send dedup
records it once. Act on a letter with a message: `Redrive { failed_message_id }` sends it again (same target,
type, payload, correlation id; once), `Discard { failed_message_id }` forgets it — over HTTP,
`POST /actors/cicada/dead-letters/messages {"type": "Redrive", "payload": {"failedMessageId": "…"}}`.

**Timers.** A timer is a delayed self-send. To stop or move it, keep the id of the pending message in state and
cancel it with `turn.cancel`: the cancel, the state change and any new timer commit together, and the turns of one
actor never overlap, so state and mailbox never disagree. There is no timer type; this is the whole recipe
([examples/src/timer.rs](examples/src/timer.rs) is the full version):

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

**Schedules.** `Schedule` (`Schedule::every(seconds)`, aligned to the epoch, and `Schedule::daily(hour, minute)`, UTC) gives a cron
actor its next slot; it is serializable, so it can live in state. See [examples/src/cron.rs](examples/src/cron.rs)
for the `Tick` chain:

```rust
use cicada::time::Timestamp;
use cicada_extras::Schedule;

let t: Timestamp = "2026-10-02T10:15:30Z".parse().unwrap();
assert_eq!(Schedule::every(3600).unwrap().next(t), "2026-10-02T11:00:00Z".parse::<Timestamp>().unwrap());
assert_eq!(Schedule::daily(2, 30).unwrap().next(t), "2026-10-03T02:30:00Z".parse::<Timestamp>().unwrap());
```

## Configuration

Node settings are the fields of `NodeConfig`, passed to the builder:
`ActorSystem::builder(store).config(NodeConfig { lease_ttl: Duration::from_secs(60), ..NodeConfig::default() })`.

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
serialization failures are re-run). [docs/operations.md](docs/operations.md) says how to size them.

## Examples

`examples/` (crate `cicada-examples`) holds ten worked examples, each with its tests under `examples/tests/`, run
with `TestActors` on virtual time:

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
| `supervision` | Supervision; failed messages become dead letters that an operator redrives. |
| `sync` | The architecture's example: a paged import, one turn per page, rescheduled hourly. |

Every example has the same shape: a public ref struct per actor type (its one field is the key), a
message enum with `#[serde(tag = "type")]`, a state struct, and an actor struct whose fields are its dependencies
(external services, faked in the tests), with one `impl Actor` whose `handle` matches on the message.

## Sample service

`sample-service` is a runnable node (binary `cicada-sample-service`, Docker image from `sample-service/Dockerfile`) hosting
the examples and the end-to-end actors, with the API and the readiness check; see
[sample-service/](sample-service/). It stops gracefully on SIGTERM and SIGINT. It is configured by environment variables:

| Variable | Meaning | Default |
| --- | --- | --- |
| `PORT` | HTTP port | 8080 |
| `NODE_ID` | node id (lease owner, shown by `/health`) | random |
| `HOSTED_TYPES` | `all`, `none`, or a comma-separated list of example groups | `all` |
| `DATABASE_URL` | PostgreSQL, `postgres://user:pass@host:5432/db`; selects PostgreSQL | — |
| `DATABASE_SCHEMA` | PostgreSQL schema of the store | `public` |
| `DATABASE_POOL_SIZE` | PostgreSQL connection pool size | the store's default (20), or `CONCURRENCY` + 4 when larger |
| `SQLITE_PATH` | SQLite file, used when `DATABASE_URL` is not set | `actors.db` |
| `API_TOKEN` | when set, the HTTP API requires `Authorization: Bearer <token>` | — |
| `LEASE_TTL`, `HEARTBEAT_INTERVAL`, `POLL_INTERVAL`, `SHUTDOWN_GRACE_PERIOD`, `MAX_TURN` | node timings, as short (`5s`, `250ms`) or ISO-8601 (`PT5S`) durations | library defaults |
| `CONCURRENCY` | maximum concurrent turns | 16 |
| `FAILURE_WATCHER_INTERVAL` | poll interval of the dead-letter watcher (group `supervision`) | `10s` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | when set, traces and metrics go there over OTLP (`ActorTelemetry`); only HTTP/protobuf, so `OTEL_EXPORTER_OTLP_PROTOCOL` must be unset or `http/protobuf` | — |

The root `docker-compose.yml` starts a PostgreSQL for development and tests
(`postgres://cicada:cicada@localhost:55432/cicada`).

## Building and testing

Requirements: Rust 1.94 or newer and, for the PostgreSQL tests, Docker.

```sh
cargo build --workspace
cargo test --workspace                     # unit, runtime, SQLite, HTTP and README tests
cargo clippy --workspace --all-targets
docker compose up -d postgres              # PostgreSQL on port 55432
CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada
PROPTEST_CASES=100000 cargo test -p cicada --lib tests::fuzz   # a long randomised run
```

Tests that need PostgreSQL are skipped when `CICADA_PG_URL` is unset. Every store backend runs the conformance suite,
which is test code of `cicada` (`crates/cicada/src/tests/conformance`), as are the randomised tests and the runtime
tests that need the store backends, which are private to the crate. `cargo test --workspace` turns every feature of
`cicada` on; CI also builds it with each feature alone.

## Using a release in another project

A release is a git tag `vX.Y.Z` with a GitHub Release; [CHANGELOG.md](CHANGELOG.md) says what changed. The crates are not
published to a registry (`publish = false`), so they are git dependencies. Name the tag once:

```toml
[workspace.dependencies]
cicada = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.11.0", features = ["postgres"] }
cicada-axum = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.11.0" }
cicada-test = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.11.0" }
```

Name the store's feature on `cicada`: `postgres`, `sqlite`, or both. None is on by default, and each builds only its
own driver; without one the crate builds and opens no store. `cicada-test` turns on the test support itself.

Every Cicada crate must come from the same tag; mixing tags gives two copies of the same types. In a project that is
not a workspace, put the same lines in `[dependencies]`, and `cicada-test` in `[dev-dependencies]`, with
`tokio = { version = "1", features = ["test-util"] }` next to it. To upgrade, change the tag and read the changelog.

While the version is 0.x, a breaking change raises the minor number (`0.3.1` to `0.4.0`) and anything else the patch
number. A breaking change is one to the public Rust API, the HTTP wire format, the store schema, a contract hash, or a
guarantee in [docs/architecture.md](docs/architecture.md). A published tag never moves.

**Access.** The repository is private, and Cargo must use the git command line to reach it. Keep the URL as it is above
everywhere: Cargo treats another URL as another source.

On a developer machine, in `~/.cargo/config.toml`:

```toml
[net]
git-fetch-with-cli = true
```

and once, `gh auth login` followed by `gh auth setup-git`.

In GitHub Actions, the default `GITHUB_TOKEN` only reaches its own repository. Create a fine-grained token with read
access to the contents of this repository only, keep it as a secret, and use it for this repository only:

```yaml
env:
  CARGO_NET_GIT_FETCH_WITH_CLI: "true"
steps:
  - uses: actions/checkout@v7
  - run: git config --global url."https://x-access-token:${CICADA_TOKEN}@github.com/helico-tech/cicada.rs".insteadOf "https://github.com/helico-tech/cicada.rs"
    env:
      CICADA_TOKEN: ${{ secrets.CICADA_READ_TOKEN }}
```

In a Docker build, pass the same token as a BuildKit secret and set it through the environment of that one step, so
that it does not end up in an image layer. The image needs `git`:

```dockerfile
RUN --mount=type=secret,id=cicada_token \
    GIT_CONFIG_COUNT=1 \
    GIT_CONFIG_KEY_0="url.https://x-access-token:$(cat /run/secrets/cicada_token)@github.com/helico-tech/cicada.rs.insteadOf" \
    GIT_CONFIG_VALUE_0="https://github.com/helico-tech/cicada.rs" \
    CARGO_NET_GIT_FETCH_WITH_CLI=true \
    cargo build --release --locked
```

and build with `docker build --secret id=cicada_token,env=CICADA_TOKEN .`.

## Decisions

Decisions, as ADRs in [docs/adr/](docs/adr/):

| ADR | Decision |
| --- | --- |
| [0001](docs/adr/0001-port-keeps-the-contract.md) | Superseded by 0008. |
| [0002](docs/adr/0002-crate-stack.md) | Crate stack: tokio, axum 0.8, sqlx 0.9 (one store per backend), serde, schemars, chrono, thiserror/anyhow, async-trait, reqwest, tracing and opentelemetry. |
| [0003](docs/adr/0003-actor-definition-api.md) | Actor definitions: a ref struct implementing `ActorRef`, a tagged serde enum, and the `Actor` trait with one `handle`; no proc macro. Its unhandled policy is gone (0018). |
| [0004](docs/adr/0004-shared-store-schema.md) | Superseded by 0008. |
| [0005](docs/adr/0005-reuse-the-console.md) | Superseded by 0011. |
| [0006](docs/adr/0006-console-in-sensus-design.md) | Superseded by 0011. |
| [0007](docs/adr/0007-container-handles.md) | Superseded by 0011. |
| [0008](docs/adr/0008-rust-only.md) | Cicada for Rust stands alone: no contract with the Kotlin implementation. |
| [0009](docs/adr/0009-one-baseline-schema.md) | The six early migrations are merged into one schema at version 1. |
| [0010](docs/adr/0010-openspec-for-work-tracking.md) | Changes, designs and tasks live in `openspec/` (OpenSpec). |
| [0011](docs/adr/0011-library-only.md) | A library only: the console (`cicada-ui`) and container handles (`cicada-docker`) are removed. |
| [0012](docs/adr/0012-slimmer-api-and-wire-format.md) | A slimmer API and wire format: turn hooks that only observe (replaced by one observer in 0018), one way to configure and send, ISO-8601 durations only, twelve endpoints. |
| [0013](docs/adr/0013-type-and-key-addresses.md) | Actors are addressed by type name and key (`sync/acme`) instead of URIs. |
| [0014](docs/adr/0014-management-reads-and-type-removal.md) | Management reads over HTTP (outcomes, stats, caused-by) and removing a type's registration. |
| [0015](docs/adr/0015-operator-actions.md) | Operator actions: node table, cancelling a running turn, pausing and purging a mailbox, retrying a message, bulk actions; the store refuses creates of an unregistered type. |
| [0016](docs/adr/0016-releases-are-tags.md) | A release is a tag on `main` with one version for the workspace; a release passes all checks first and never moves. |
| [0017](docs/adr/0017-contracts-ignore-descriptions-and-stores-refuse-foreign-databases.md) | A contract hash leaves out the schemas' descriptions (the published contract keeps them); the SQLite store refuses a database that holds other tables. |
| [0018](docs/adr/0018-a-smaller-core-layers-and-watches.md) | A smaller core (every actor has state, one-name keys, one turn observer in place of three hooks, no public store trait), layers above it (`cicada-app`, `cicada-actors`), JSON Schema checks at the HTTP door, watches and control effects. |
| [0019](docs/adr/0019-child-actors.md) | Child actors: an actor may have a parent, the actor whose turn created it, and removing the parent removes what it owns. |
| [0020](docs/adr/0020-the-change-loop.md) | The change loop: one order of changes, each described with the owner one ahead, built and validated in the background, and released when it lands. |
| [0021](docs/adr/0021-a-key-is-one-name.md) | A key is one name: a ref is written as one string or a unit, a `:` is an ordinary character, contracts hold no key fields, and a database is upgraded in place. |
| [0022](docs/adr/0022-typed-errors-and-no-such-type.md) | Typed errors: a named error per module, a start error with a problem per cause (a contract conflict is its own case), a send to an unregistered type answers `404 no-such-type`, and a migration that violates a constraint is an invalid store. |
| [0023](docs/adr/0023-the-rules-of-a-watch.md) | The rules of a watch: notifications in the watcher's own family, a type target at a dot and `*` for every type (failures only), `Stopped` with its cause and `by`, a notification marked and named after its event, and G2 and G4 that name the notifications. |
| [0024](docs/adr/0024-control-effects.md) | Control effects: a turn pauses, resumes, purges and deletes another actor with the meaning of the operator's calls, an effect on an actor that is not there does nothing, a turn cannot delete itself or an ancestor, deletes come before creates (a restart), and G1, G2 and S7 that say so. |
| [0025](docs/adr/0025-the-claim-limit.md) | The claim limit: a message counts its lost claims (a claim that another claim replaced although its turn neither committed nor gave the lease back), a node fails a message with as many as its limit (5 by default) through a commit that tells its watchers, and G4 that names the limit. |
| [0026](docs/adr/0026-the-schema-check-at-the-http-door.md) | The schema check at the HTTP door: a send and a create over HTTP are checked against the registered schema on every node and refused with `422` (not `400` as 0018 said), a hosting node still decodes, a repeated send is answered first, no format is asserted, a schema the validator cannot use is not enforced, and JSON that code sends is not checked. |

Design questions and their answers:

| Question | Decision |
| --- | --- |
| Crash loops | The claim limit in the core, on by default (ADR 0025): a message with 5 lost claims ends `failed`; `lostClaims` is on the envelope and in `/messages/{id}`. |
| Contract changes | Strict contract-hash equality; a node with a different contract for a registered type name fails to start. Contracts include the generated schemas, so a schemars upgrade can change hashes. Doc comments (`description`) do not count in the hash. |
| State evolution | No migrations; changes must be additive with defaults (`#[serde(default)]`). |
| Observer data | `headers` on the envelope, which an observer may add to a turn's sends; `correlation_id` is kept (the HTTP contract uses it). |
| Send to an absent instance inside a turn | The whole turn fails at commit: outcome `failed` with the reason, no effects. |
| Admin delete | Allowed: `DELETE /actors/{type}/{key}`. |
| One-way messaging | Kept; no ask/await. |
| Defaults | Lease 30 s, heartbeat 10 s, poll 500 ms, max turn 15 min, retention 7 days; all configurable. |

Design notes:

- **Definitions are checked by the compiler.** One `handle` with an exhaustive `match` over the messages its ref
  names, which are declared once, on the ref (ADR 0003, ADR 0018). Handlers return `Result<(), HandlerError>`:
  `HandlerError::permanent` / `turn.fail(..)` refuse further retries, and a panic is caught and treated as a failure
  (`panic: …`).
- **The host owns the node.** The host builds an `ActorSystem`, calls `start` and `stop` itself, merges `actor_api()`
  into its router, and owns the store. `actor_api()` takes the `ActorSystem` itself, so it cannot be mounted without a
  node.
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
  propagator's empty `tracestate` is left out.
- **PostgreSQL settings.** Connection parameters go in the URL; no type of the SQL driver is in the public API, so a
  setting a URL cannot carry is added to `PostgresOptions` when a host needs it (ADR 0018). `socket_timeout` bounds a whole transaction attempt (the
  connection is then closed), not every read, because sqlx has no per-read timeout.
- **Cancelling `run_due_turns`.** Dropping its future cancels the turns like a node stop after the grace period: the
  worker is stopped first, then the lease is released only when the handler had not ended, and claims that arrive
  after the caller is gone are released.
- **Errors.** Error texts use Rust names (`max_lost_claims must be at least 1, or None for no limit`); a SQLite file
  of a newer schema is refused with `StoreError::Invalid`.

Known limitations: the claim limit cannot tell the message that kills a node from the messages that ran beside it,
and a node killed hard as a matter of routine (a deploy that sends SIGKILL) loses the claims of its turns too; external
side effects are at-least-once (use `message().id` as idempotency key, also across retries); a commit rejected because
of the handler's own sends is not retryable; the client has no streaming of message status (poll `GET /messages/{id}`).
