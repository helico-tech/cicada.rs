# Cicada for Rust — durable actors

Cicada is a small durable actor system for Rust services. An actor is a row of state plus a mailbox in a shared
transactional store (PostgreSQL or SQLite); it occupies memory only while one of its messages is being handled. Every
service that runs an `ActorSystem` is a *node*. Nodes are peers that coordinate only through the store: there is no
coordinator and no cluster protocol.

This is a port of [Cicada for Kotlin](https://github.com/helico-tech/cicada). It keeps that implementation's contract
(guarantees, store schema, HTTP API, console), so Kotlin and Rust nodes can share one database and serve each other's
callers. See [Interoperability with Kotlin nodes](#interoperability-with-kotlin-nodes).

- **Durable turns.** Claim → load → handle → commit. New state, outgoing messages, creates and the message's outcome
  are written in one transaction; a node that dies mid-turn loses its lease and the turn is re-run elsewhere.
- **One turn at a time per actor**, messages in delivery-time order, delayed messages never early, exactly one
  outcome per message (`done`, `failed`, `cancelled`, `purged`).
- **Typed and checked.** Actor addresses are typed refs bound to URI templates (`actor://sync/{tenant}`); the compiler
  checks that a ref accepts a message, and that a handler covers every message of its family.
- **One HTTP API** (fourteen endpoints) on every node as an axum `Router`, a typed client, an in-process API,
  `TestActors` on tokio's paused time, and per-message retry policies set by the sender.

The design — domain language, guarantees G1–G6, store requirements S1–S10, the HTTP contract — is in
[docs/architecture.md](docs/architecture.md); the decisions of the port in [docs/adr/](docs/adr/); how to size,
deploy, upgrade and watch nodes in [docs/operations.md](docs/operations.md). The most important decisions and the
known differences from Kotlin are summarised [below](#decisions-and-deviations).

## Crates

| Crate | Contents | Kotlin module |
| --- | --- | --- |
| `cicada` | Addressing (`ActorAddress`, `ActorRef`, `UriTemplate`), the definition API (the `Actor` trait), the runtime and dispatch loop (`ActorSystem`) with per-message retry (`RetryPolicy`) and retention (`Retention`), the `Store` trait, plugin hooks, contracts and JSON Schemas, the HTTP wire model (`cicada::api`). No HTTP server. | `actors-core` |
| `cicada-store-sqlite` | `SqliteStore` (sqlx): one file, WAL, one writer. For a single node or a few processes on one host. | `actors-store-sqlite` |
| `cicada-store-postgres` | `PostgresStore` (sqlx): `FOR UPDATE SKIP LOCKED` claims, `LISTEN/NOTIFY` wake-ups. For many nodes. | `actors-store-postgres` |
| `cicada-store-conformance` | The shared suite (S1–S10 and robustness cases) every store must pass, as a macro. | `actors-store-conformance` |
| `cicada-axum` | `actor_api()` (the fourteen endpoints) and `actor_health()` (a public readiness check) as axum routers. | `actors-ktor` |
| `cicada-ui` | The actor console: the React app of the Kotlin repo, embedded and served by `actor_ui()`. | `actors-ui` |
| `cicada-client` | `ActorClient`: typed HTTP client (reqwest) for remote callers, idempotent sends. | `actors-client` |
| `cicada-test` | `TestActors`, `InMemoryStore`, virtual time, contract snapshots and state samples. | `actors-test` |
| `cicada-extras` | Building blocks on the public API, no core changes: `CrashLoopGuard`, dead letters (`DeadLettersActor`, `FailureWatcher`), `Timer`, `Schedule`. | `actors-extras` |
| `cicada-otel` | `ActorTelemetry`: OpenTelemetry traces and metrics per turn, a plugin on the OpenTelemetry API. | `actors-otel` |
| `examples` (`cicada-examples`) | Ten worked examples, each tested with `TestActors`; see [Examples](#examples). | `examples` |
| `sample-service` | A runnable node hosting the examples, configured by environment variables. | `sample-service` |

## Quick start

### Define an actor type

A ref struct binds a URI template; an enum with `#[serde(tag = "type")]` holds the messages (the variant name is the
type name on the wire); one `handle` matches on the message, so the compiler checks that every message is handled.
This is [examples/src/sync.rs](examples/src/sync.rs), shortened:

```rust
use std::future::Future;
use std::time::Duration;

use cicada::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sync {
    pub tenant: String, // the fields are the template's parameters
}

impl ActorRef for Sync {
    const TEMPLATE: &'static str = "actor://sync/{tenant}";
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
    const TEMPLATE: &'static str = "actor://reports/{tenant}";
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
    type Message = SyncMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail; // mandatory: Fail | Drop | Custom
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

Inside a handler, `turn` offers:

- `self_ref()` (the typed ref), `self_address()`, `state()`, `state_mut()` and `set_state(..)` (taking the state
  mutably marks it written; it is stored at commit);
- `message()`, the envelope: `id`, `to`, `from`, `message_type`, `payload`, `attempt`, `failures`, `last_error`,
  `retry`, `retention`, `caused_by`, `deliver_at`, `correlation_id`, `headers`;
- `now()` (store time at claim; it does not move during the turn), `current_time()` (`now` plus the time the turn has
  run so far), `attributes()` (shared with plugins);
- `send(&ref, msg)`, `send_with(&ref, msg, SendOptions)` and `send_self(..)` / `send_self_with(..)`. `SendOptions`
  sets `delay` or `at`, `correlation_id`, `header`, `retry`, `retention` and `id`. Each returns the new message's id.
- `send_to(&address, &msg, options).await` and `send_raw(&address, type, payload, options).await` for raw addresses
  such as `message().from`; the type name is checked against the type registry when called;
- `cancel(&id)` (a pending message of this instance gets `cancelled` at commit; keep the id a send returns to replace
  a scheduled message), `create(&ref, &initial_state)` / `create_stateless(&ref)` (applied before the turn's sends),
  `stop()`, and `fail(reason)`, used as `return Err(turn.fail("declined"))` (ends the message `failed` now, never
  retried).

Effects are buffered and committed atomically. A handler that returns `Err` (any error converts with `?`) or panics
commits `failed` and nothing else — unless the message carries a [retry policy](#retry). Sends refuse a header with
the reserved prefix `cicada-` and fields beyond the send limits with `TurnError::Invalid`, which the handler may
handle. A stateless type has `type State = Stateless`.

**Times in long turns.** A `delay` counts from the commit; an `at` is absolute. In a turn that runs for 70 s,
`SendOptions::new().at(turn.now() + 1 minute)` is due 10 s *before* the commit, so it runs right after it: use
`turn.current_time() + 1 minute` (or `delay`) for a pause after the work.

**Blocking code.** Handlers run as futures on the tokio runtime. The turn timeout, a lost lease and a node stop cancel
a handler at its next `.await`; code that does not yield (blocking I/O, `std::thread::sleep`, heavy computation) runs
on until it returns and holds a runtime worker thread meanwhile. Run it with `tokio::task::spawn_blocking`.

### Run it in an axum service

The host owns the store and the node's life cycle; the HTTP API is a `Router` it mounts with its own
authentication:

```rust,no_run
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use cicada::store::Store;
use cicada::{ActorSystem, SendResult};
use cicada_axum::{actor_api, actor_health};
use cicada_store_postgres::{PostgresStore, PostgresStoreConfig};
use cicada_store_sqlite::{SqliteOptions, SqliteStore};
# use cicada_examples::sync::{Page, ReportsActor, Sync, SyncActor, SyncApi, SyncMsg};
# struct HttpSyncApi;
# impl SyncApi for HttpSyncApi {
#     async fn fetch(&self, _: &str, _: Option<&str>, _: &str) -> Page { Page { size: 0, next: None } }
# }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store: Arc<dyn Store> = match std::env::var("DATABASE_URL") {
        Ok(url) => Arc::new(PostgresStore::connect(&url, PostgresStoreConfig::default()).await?),
        Err(_) => Arc::new(SqliteStore::open("actors.db", SqliteOptions::default()).await?),
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
        .merge(actor_api(system.clone()).layer(middleware::from_fn_with_state(token, bearer))) // the fourteen endpoints
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
  from the one already registered for its template, or a template that overlaps another type's fails the start with
  a `StartError` that lists every problem. No address may match two templates (`actor://jobs/{id}` and
  `actor://jobs/cleanup` both match `actor://jobs/cleanup`). `start` then runs the dispatch loop, the retention
  sweep and, on PostgreSQL, the collection of wake-ups.
- `system.stop()` stops claiming, lets running turns finish within `shutdown_grace_period` (30 s), then cancels the
  rest and releases their leases. While it stops, `/health` and `/ready` answer `503`. `stop` is idempotent.
- The store is the caller's: `ActorSystem` never closes it. Open it before `start` and close it after `stop`.
  `PostgresStore::connect(url, config)` accepts the libpq form of `DATABASE_URL`
  (`postgres://user:pass@host:5432/db?sslmode=require`) and the Kotlin JDBC form (`jdbc:postgresql://…`);
  `connect_as(url, user, password, config)` adds credentials, and `connect_with(PgConnectOptions, config)` takes sqlx
  options built by the caller (TLS client certificates and the like).
- A node hosting no types still serves the whole API (create, send, query) for every registered type.
- `actor_api(system)` and `actor_health(system, path)` are plain routers with no fallback: `merge` them or `nest`
  them under a prefix, and wrap them in your own layers. They need nothing installed on the app: they read and write
  JSON themselves with the wire types of `cicada::api`. `actor_api_with(system, ApiConfig { max_request_body_bytes })`
  changes the body limit.
- `ActorSystem` is cheap to clone; every clone is the same node. Its methods are the in-process API: `create`,
  `send`, `send_with`, `send_to`, `send_message`, `get_actor`, `delete_actor`, `list_actors`, `pending`,
  `list_pending`, `status`, `outcome`, `chain`, `cancel`, `failures`, `types`, `type_of`, `health`.
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

```rust,no_run
use cicada_client::ActorClient;
# use cicada_examples::sync::{Reports, ReportsState, Sync, SyncMsg, SyncState};
# async fn example() -> Result<(), cicada_client::ClientError> {

let client = ActorClient::new("http://localhost:8080"); // or ActorClient::with_client(url, reqwest_client)
client.create(&Reports { tenant: "acme".into() }, &ReportsState::default()).await?;
let state = SyncState { source: "https://api.acme.test/items".into(), imported: 0 };
client.create(&Sync { tenant: "acme".into() }, &state).await?;
let receipt = client.send(&Sync { tenant: "acme".into() }, SyncMsg::Run).await?; // client-generated id, retried with it
let status = client.status(&receipt.id).await?; // Pending, Running, Done, Failed, Cancelled, Purged
println!("{:?}", status.map(|s| s.status));
# Ok(()) }
```

The client takes the same ref and message types as the actor definitions. Authentication is the `reqwest::Client`'s:
give `ActorClient::with_client` one with default headers. Expected alternatives are values (a missing instance is
`None`, a duplicate send is `receipt.duplicate`, a cancel returns a `CancelOutcome`); every other error response is
`ClientError::Api` with the server's problem. A send that fails on I/O or with `503` is retried with the same id (3
attempts by default), so it is enqueued at most once. The client also talks to Kotlin nodes.

The Rust snippets in this README are doctests of the `examples` crate (`cargo test -p cicada-examples --doc`), so
they keep compiling, and the ones that need no server or database run.

## HTTP API

`{path}` is the actor URI without `actor://`, segments percent-encoded exactly as in the URI (`actor://a/b%20c` →
`/actors/a/b%20c`). Errors are `application/problem+json` (RFC 9457) with `type` `urn:cicada:problem:<slug>` and the
request's path and query as `instance`.

| Endpoint | Success | Errors |
| --- | --- | --- |
| `PUT /actors/{path}` body `{state?}` — create | 201 created; 200 already exists (untouched) | 404 no type matches; 422 state cannot be decoded, a state for a stateless type, or non-canonical parameters (hosting node) |
| `GET /actors/{path}` — type, params, state, version, createdAt | 200 | 404 |
| `DELETE /actors/{path}` — admin delete, purges the mailbox | 204 | 404 |
| `GET /actors?prefix=&cursor=&limit=` — instance URIs, paged | 200 | |
| `POST /actors/{path}/messages` body `{id?, type, payload?, deliverAt? \| delay?, correlationId?, headers?, retry?, retention?}` — send | 202 `{id, deliverAt}`; 200 that id was already accepted (same target and type) | 400 invalid `retry` or `retention` policy; 404 no instance; 409 id already used for another target or type; 422 unknown message type, undecodable payload (hosting node only), reserved `cicada-*` header |
| `GET /actors/{path}/messages?cursor=&limit=` — pending mailbox in delivery order, each with `attempt`, `failures`, `lastError`, `retry` | 200 | 404 |
| `GET /messages?prefix=&cursor=&limit=` — pending messages of every instance under the prefix (including running ones), instance by instance in URI order, each in delivery order | 200 | |
| `GET /messages/{id}` — status, error, `failures` and `lastError`, node and lease while running, envelope (with `retry`, `retention` and `causedBy`) | 200 | 404 unknown, past retention, or removed by its retention policy |
| `GET /messages/{id}/chain` — the messages that led to this one and the ones it caused (see [Message chains](#message-chains)) | 200 | 404 unknown or past retention |
| `DELETE /messages/{id}` — cancel a pending message | 204 | 404; 409 running or already finished |
| `GET /failures?prefix=&since=&cursor=&limit=` — failed messages with envelope and error | 200 | |
| `GET /types` — templates, contract hashes, state and message JSON Schemas | 200 | |
| `GET /types?uri=` — the type matching one URI, with its params | 200 | 404 |
| `GET /health` — node running and store reachable | 200 | 503 |

On every endpoint, additionally:

- `400` — malformed JSON, path or query; an unknown key in a request envelope (`{"paylaod": …}`, also inside `retry`
  and `retention`); both `deliverAt` and `delay`; a delivery time beyond year 9999; envelope limits (id/type ≤ 200
  chars, ≤ 64 headers, no NUL); an invalid retry policy; `limit` outside 1..1000; a bad cursor. Payloads and states
  themselves are decoded leniently (unknown keys ignored).
- `405` — `POST /actors/{path}` without `/messages`.
- `413 request-too-large` — a body larger than `ApiConfig::max_request_body_bytes` (default 1 MiB), by its
  `Content-Length` or while reading.
- `503 store-unavailable` — the store cannot be reached (worth retrying; the client retries sends with the same id).
- `500 internal-error` — anything unexpected, such as stored data that no longer decodes, or a panic (not worth
  retrying as such).
- `401`/`403` — whatever the host's authentication layer answers.

`delay` is an ISO-8601 duration (`"PT5S"`; `"5s"` also works) or a number of milliseconds; `deliverAt` is an ISO-8601
instant. Both are resolved against the store clock. Listings take `cursor` (the previous page's `next`) and `limit`
(default 100); a `prefix` is an actor-URI prefix (`actor://sync/`; the scheme may be left out). Pages are a keyset,
not a snapshot: in a mailbox listing, a message rescheduled in place between two pages (later `deliverAt`, new `seq`)
can appear on both, so deduplicate by id. A trailing segment that is literally `messages` names the mailbox; an
instance whose last segment is `messages` is addressed as `%6Dessages` (`ActorAddress::to_http_path` does this).

## Actor console

`cicada-ui` is a web console over the HTTP API, for developers and on-call engineers. Merge it next to the API:

```rust
# fn app(system: cicada::ActorSystem) -> axum::Router {
use axum::Router;
use cicada_axum::actor_api;
use cicada_ui::actor_ui;

Router::new()
    .merge(actor_api(system))         // the API, at the root (behind your authentication layer)
    .merge(actor_ui("/console", "/")) // the console, at /console/
# }
```

`actor_ui(mount_path, api_path)` takes where the console is served and where `actor_api()` is mounted on the same
origin (`try_actor_ui` returns an error instead of panicking on an invalid path). The console is the React +
TypeScript single-page app of the Kotlin repo (ADR 0005), built with Vite when `cicada-ui` compiles (Node.js 22+ and
npm; `CICADA_UI_SKIP_BUILD` embeds an empty console) and embedded in the binary with `config.json`; it calls only the
API, from the browser. What it does:

- **Actors** — list by type template or URI prefix, cursor pages; create (form from the state's JSON Schema, or raw
  JSON) and delete (type the URI to confirm).
- **Actor** — type, params, version, state as collapsible JSON; the pending mailbox with cancel; recent failures.
- **Pending** — the pending messages of every actor, by type, URI prefix and message type.
- **Message** — status, lease or outcome, retries and policy, persistence policy, envelope, payload; cancel while
  pending. The **message chain**: what led to the message and everything it caused, as a tree graph (pan and zoom,
  click a card to open that message) or an indented list; removed records show as removed.
- **Send** — payload form from the message type's JSON Schema (or raw JSON), now / after a delay / at a time, id,
  correlation id, headers, retry policy, persistence policy (done / failed: default, remove, keep for a time, keep the
  last N). A generated id is reused by Retry Now, so a retried send cannot enqueue twice.
- **Failures** — newest first, by prefix, message type and time window. **Types** — templates, hashes, schemas.
- **Live** — the API has no push, so the open page is polled every 5 s (paused while the tab is hidden, or with the
  Live switch). New rows, a version that went up, changed state fields and messages that left the mailbox are
  highlighted for one refresh. The top bar shows the node's health; ⌘K opens an actor URI or message id.
- **Light and dark** — follows the system setting until the viewer picks one with the top bar's switch, which this
  browser then remembers.

Limits that come from the API: the actor list carries only URIs (type and params are matched to the templates in the
browser); finished messages are not listed, only found by id or under Failures; the Pending page does not mark running
messages (open one to see); `/failures` lists failed outcomes only, oldest first, so the console loads the whole
window (at most 10 000) and sorts it. Authentication stays the host's: cookies and HTTP Basic credentials the browser
has are sent; for a bearer-token API the console asks for the token on the first `401` and keeps it in the tab's
session storage. The page sends a strict Content Security Policy and inserts every value from the node as text. No
fonts are loaded from outside.

**Types checked end to end.** The console parses every response with zod schemas generated from the Kotlin wire
classes. `crates/cicada-ui/tests/api_fixtures.rs` records a response of every endpoint from a running Rust node and
runs the console's fixture test over them, so a Rust response the console does not expect fails the build.

## Testing with `TestActors`

`cicada-test` runs types against an in-memory store whose clock is tokio's paused time, so delays, timers,
heartbeats and turn timeouts are deterministic and instant. Run each test with `#[tokio::test(start_paused = true)]`
(tokio's `test-util` feature):

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

# #[tokio::main(flavor = "current_thread", start_paused = true)]
# async fn main() {
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
# }
```

`TestActors` also offers `create_stateless`, `send_with`, `send_raw` (the raw result, refusals included),
`run_until_idle_max`, `pending_at`, `list(prefix)`, `exists`, `state_json`, `version`, `outcome(&message)`, `now`,
`clock`, and `system()` / `store()` for anything else (every `ActorSystem` call). The builder's `with(|node| …)`
configures the node like `ActorSystem::builder` (hosted types, plugins, `default_max_turn`, …); `start_at` and
`retention` set the store's start time and retention. Nothing runs in the background: turns run only inside
`run_until_idle` and `advance_time_by`.

A dependency that needs a clock or a timer of its own (a rate limiter, a cache with expiry) uses `tokio::time`, which
is the test's virtual time; `test.clock()` is the store's virtual wall clock.

### Contract snapshots

A node refuses to start when a type's contract differs from the one registered for its template, which in a rolling
deploy shows up in production. `assert_contracts_match` moves that failure into a test: it keeps each type's contract
as a JSON file and fails, naming what changed (state schema, message types added, removed or changed), when the code
no longer matches. In a test:

```rust,no_run
use cicada::actor::contract_of;
use cicada_test::assert_contracts_match;
# use cicada_examples::sync::{Page, ReportsActor, SyncActor, SyncApi};
# struct FakeApi;
# impl SyncApi for FakeApi {
#     async fn fetch(&self, _: &str, _: Option<&str>, _: &str) -> Page { Page { size: 0, next: None } }
# }

assert_contracts_match("tests/contracts", &[
    contract_of::<SyncActor<FakeApi>>().unwrap(), // fails with the type's problems when it is invalid
    contract_of::<ReportsActor>().unwrap(),
]);
```

A missing file is written (commit it). To accept a change, delete the file or run the tests with
`CICADA_UPDATE_CONTRACTS=true`. The files are named after the template (`actor://sync/{tenant}` →
`sync_{tenant}.json`) and hold the contract as `GET /types` serves it.

### State samples

State that no longer decodes fails every turn of its instance. Keep samples of the shapes a state ever had and check
them with the runtime's own decoder (serde: unknown keys are ignored, missing fields need `#[serde(default)]`):

```rust,no_run
use cicada_test::{assert_state_decodes, record_state_sample};
# use cicada_examples::sync::SyncState;

let dir = "tests/states/sync";
let first_run = SyncState { source: "https://api.acme.test/items".into(), imported: 0 };
record_state_sample(dir, "first-run", &first_run); // written once, never overwritten
assert_state_decodes::<SyncState>(dir); // every sample in the directory, also those of earlier versions
```

Decoding is all it checks: a sample whose renamed field is now ignored still decodes, with the default in its place.

## Retry

Retry is part of a message's **delivery**, set by its **sender** next to the delivery time — like a workflow engine's
caller choosing the retry policy of an activity. Actor types and plugins do not configure it, and a message sent
without a policy is never retried: a handler that fails ends it `failed`.

```rust
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada_examples::saga::{Order, OrderActor, PaymentGateway, Payments, PaymentsActor, PaymentsMsg};
// Sender (the Order saga, examples/src/saga/order.rs): this delivery gets up to 4 handler runs.
# fn on_reserved(turn: &mut Turn<OrderActor>, me: Order, amount_cents: i64) -> HandlerResult {
let backoff = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(30)) // 1 s, 2 s, 4 s, … or Backoff::fixed
    .expect("a valid backoff");
let retry = RetryPolicy::new(4, backoff).expect("a valid policy"); // handler runs, including the first
turn.send_with(&Payments, PaymentsMsg::Charge { reply_to: me, amount_cents }, SendOptions::new().retry(retry))?;
# Ok(())
# }

// Receiver (Payments): an error is retried under the sender's policy; turn.fail(..) is a veto.
# async fn on_charge<G: PaymentGateway>(
#     turn: &mut Turn<PaymentsActor<G>>, gateway: &G, reply_to: Order, amount_cents: i64,
# ) -> HandlerResult {
if amount_cents <= 0 {
    return Err(turn.fail(format!("invalid amount {amount_cents}"))); // permanent: failed now, never retried
}
// A retry runs the same message again, so its id is stable across attempts: the right idempotency key.
let result = gateway.charge(&reply_to.id, amount_cents, turn.message().id.as_str()).await?; // Err: retried
# let _ = result;
# Ok(())
# }
# fn main() {}
```

Every send takes `retry` through `SendOptions::retry`: `send_with` in a handler (typed and raw), the `ActorSystem`
sends, `ActorClient::send_with` / `send_message`, `TestActors::send_with`, and over HTTP the `retry` field of
`POST /actors/{path}/messages`:

```sh
curl -X POST localhost:8080/actors/payments/messages -d '{"type": "Charge", "payload": {…},
  "retry": {"maxAttempts": 4, "backoff": {"kind": "exponential", "initial": "PT1S", "factor": 2.0, "max": "PT30S"}}}'
# or "backoff": {"kind": "fixed", "delay": "PT5S"}; durations as ISO-8601 (also "5s" or milliseconds); factor defaults to 2
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
succeeds, `failed` with the handler's error when the policy is exhausted. `/failures` lists only such final failures.
The handler's buffered effects are discarded on every failed run, as for any failure. Cancel works on a waiting
message; deleting or stopping the instance purges it.

- **The receiver's veto.** A handler that returns `HandlerError::permanent(reason)` (or `turn.fail(reason)`, the same
  thing) ends the message `failed` at once with that reason, whatever the policy. `HandlerError::permanent_from(err)`
  keeps the cause.
- **Not retried:** undecodable state, the unhandled policies `Fail` and `Drop` and a custom policy's
  `UnhandledDecision::Fail` / `Drop` (decisions, not failures: no `on_handler_error` either), failing `on_claim` /
  `before_commit` hooks, and commits rejected by the store (e.g. a send to an absent instance). A *custom* unhandled
  policy that returns `Err`, panics or times out is a handler failure and follows the message's policy.
- **Timeouts.** A run cut off at its maximum turn is a failure retried per policy. A handler that blocks its thread
  past the maximum (code that does not yield) is waited for; it is retried only if it then fails with an ordinary
  error. If it returns `Ok`, vetoes or (unhandled policy) decides, the message ends `failed` with `timeout`, never
  re-run.
- **Counting.** `message().failures` counts failed runs (this run is attempt `failures + 1`); `message().attempt`
  counts claims, including crash recoveries, which do not use up an attempt.
- **Forwarding.** `message().retry` is the incoming policy; pass it on with
  `SendOptions { retry: turn.message().retry.clone(), ..SendOptions::new() }`.
- **Dedup.** A resend with an id that was already accepted is `200` / `SendResult::Duplicate` when target and type
  match, as before; a different `retry` on it is ignored, like a different payload — the first accepted send stands.
- **Hooks.** The runtime decides retry-or-fail first; then `on_handler_error` hooks run and see the decision
  (`will_retry`, `retry_delay`, `outcome`, `failure_reason`). They may add effects (committed with the retry too) or
  replace the failure with a final outcome (`set_outcome(TurnOutcome::Done)` with compensating sends, or
  `TurnOutcome::Failed(…)`), which cancels the retry. They cannot schedule a retry: `set_outcome` refuses any other
  reschedule. `on_outcome` sees `TurnOutcome::Rescheduled` for each retried run.
- **Unreadable policies.** A stored or served policy that a node or client cannot decode (a backoff kind of a newer
  version, a damaged row) is logged as a warning and read as no policy: the message is handled without retry, and
  claims, status reads and listings keep working.
- A backoff never exceeds `MAX_DELAY`; a retry the store still cannot schedule (due past `cicada::time::MAX_INSTANT`)
  ends `failed` with the handler's error and the refusal.

`RetryPolicy` and `Retention` have JSON Schemas, so a message or state may carry one as a field.

## Message persistence

How long a finished message stays readable is also part of its delivery, chosen by the sender next to `retry`: a
`Retention` policy with one rule for a message that ends `done` and one for a message that ends `failed`. A missing
rule, and the outcomes `cancelled` and `purged`, keep the store default (the store's retention, 7 days by default).

```rust
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada_examples::saga::{Order, Payments, PaymentsMsg};
# async fn example(system: &ActorSystem, order: Order) -> anyhow::Result<()> {
let charge = || PaymentsMsg::Charge { reply_to: order.clone(), amount_cents: 900 };
let gone_when_done = SendOptions::new().retention(Retention::REMOVE_ON_DONE); // gone once it succeeded
system.send_with(&Payments, charge(), gone_when_done).await?;

let retention = Retention {
    done: Some(Keep::last(100).expect("a valid rule")), // the 100 most recent done charges of this instance
    failed: Some(Keep::keep_for(Duration::from_secs(90 * 24 * 3600)).expect("a valid rule")), // 90 days
};
system.send_with(&Payments, charge(), SendOptions::new().retention(retention)).await?;
# Ok(())
# }
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
- A removed record is gone from `GET /messages/{id}` (404), `/failures` and cancel. Its content is dropped at once
  (`Remove`) or by the next retention sweep (`sweep_interval`, 1 h by default).
- **Dedup and chains keep working.** The store keeps a minimal record (id, target, sender, type, status, times,
  cause) for its retention, which is also the dedup window: a resend with a removed message's id is still `200`, so a
  client retry after a lost acknowledgement never runs the message twice. Message chains show it as removed.
- Bounds: `count` in 1..10 000; `age` positive and at most `MAX_DELAY`; `Keep::last` and `Keep::keep_for` check them.
  Invalid policies are `400` over HTTP. A resend of an accepted id with another policy is a duplicate; the first
  policy stands. Unreadable stored policies read as none. Every send path takes `retention`, as for `retry`;
  `message().retention` lets a handler pass it on.

## Message chains

Every message sent from a turn records the id of the message that turn handled as `caused_by` (`None` for a send
from outside an actor). `GET /messages/{id}/chain` (`ActorSystem::chain`, `ActorClient::chain`) follows these links:
up to the message that started it all, and down through everything the message caused, directly or not. The result
is a flat list of nodes (`id`, `causedBy`, `to`, `from`, `type`, `status`, `deliverAt`, `finishedAt`, `error`,
`attempt`, `failures`, `removed`) plus `truncated`, which is true beyond 50 steps either way or 500 caused messages
(`MessageChain::MAX_DEPTH`, `MAX_DESCENDANTS`). Messages past retention are left out, so a chain can start at a
message whose cause is gone. The console draws it as a tree on the message page.

## Plugins

A plugin observes or alters a turn through four async hooks of the `Plugin` trait: `on_claim` (after load, before the
handler; an `Err` fails the turn), `on_handler_error` (sees the retry decision; may add effects or replace the
outcome), `before_commit` (may add effects; an `Err` fails the turn) and `on_outcome` (after a successful commit).
Effects a hook adds join the turn's single commit, so the guarantees hold with or without plugins. The plugin value
is its configuration; the turn's `Attributes` (a bag keyed by type) carry data from one hook to the next and to the
handler.

```rust
use cicada::plugin::{HookResult, OutcomeContext, Plugin};
# use std::sync::Arc;
# use cicada::{ActorSystem, ActorType};
# use cicada_examples::sync::ReportsActor;

struct Audit {
    prefix: String,
}

#[async_trait::async_trait]
impl Plugin for Audit {
    fn name(&self) -> &str {
        "Audit"
    }

    async fn on_outcome(&self, ctx: &OutcomeContext<'_>) -> HookResult {
        println!("{}: {} -> {:?}", self.prefix, ctx.info().message().id, ctx.outcome());
        Ok(())
    }
}

# fn node(store: Arc<dyn cicada::store::Store>) {
let system = ActorSystem::builder(store)
    .install(Audit { prefix: "node".into() }) // every hosted type
    .host(ActorType::new(ReportsActor).install(Audit { prefix: "reports".into() })) // this type; wins over the node's
    .build();
# }
```

Node plugins run before type plugins, each in installation order. A plugin installed on both the node and a type runs
once for that type, with the type's installation (matched by `name`). `ActorType::new(actor).max_turn(d)` also sets
a type's maximum turn, overriding `Actor::MAX_TURN`. Hook headers may use the reserved prefix `cicada-`; handlers and outside
senders may not. The table of which hook runs in which situation is in the `cicada::plugin` docs.

### OpenTelemetry

`cicada-otel` adds OpenTelemetry tracing and metrics as a plugin. It depends on the OpenTelemetry API only; the
service brings the SDK and exporters (`opentelemetry_sdk`, `opentelemetry-otlp`) and installs them as the global
providers, or passes them to `ActorTelemetry::new(&tracer_provider, &meter_provider)`:

```rust
# use std::sync::Arc;
# use cicada::{ActorSystem, SendOptions};
# use cicada_examples::sync::{Sync, SyncMsg};
use cicada_otel::{ActorTelemetry, current_trace_headers};
# async fn node(store: Arc<dyn cicada::store::Store>) -> anyhow::Result<()> {

let system = ActorSystem::builder(store).install(ActorTelemetry::global()).build(); // the global providers
// Outside an actor, the caller's trace context goes into the message's headers:
let options = SendOptions { headers: current_trace_headers(), ..SendOptions::new() };
system.send_with(&Sync { tenant: "acme".into() }, SyncMsg::Run, options).await?;
# Ok(())
# }
```

- **A span per turn**, `process <template>` (kind `CONSUMER`), with `messaging.*` and `cicada.*` attributes (actor,
  message type, attempt, failures, outcome, messages sent). `failed` turns have status `ERROR`; a handler error is an
  exception event, also when the message is retried. A turn that never commits (lost lease, node stopped mid-turn)
  ends its span with `cicada.outcome` = `abandoned`.
- **One trace across actors.** The span's parent is the trace context in the message's headers (W3C `traceparent`).
  Plugins cannot change a handler's sends, so a handler passes it on itself:
  `SendOptions { headers: cicada_otel::trace_headers(turn), ..SendOptions::new() }` (the saga example does). Over
  HTTP, put `traceparent` in the message's `headers`. `with_propagator(..)` replaces the global propagator.
- **Metrics:** `cicada.turn.duration` (claim to commit) and `cicada.message.lag` (due time to claim), histograms in
  seconds by `cicada.actor.type` and `cicada.message.type`; the duration also by `cicada.outcome`.

## Extras

`cicada-extras` holds building blocks that use only the public API (plugin hooks and the operations of
`ActorSystem` or `ActorClient`).

**Crash-loop guard.** A message that kills its node (out of memory, `std::process::exit`, an abort) is otherwise
claimed again forever. `CrashLoopGuard` counts the claims of a message that ended without a commit
(`attempt - failures - 1`) and, at the limit, fails the message in `on_claim` before the handler runs:

```rust
# use std::sync::Arc;
# use cicada::ActorSystem;
use cicada_extras::CrashLoopGuard;
# fn node(store: Arc<dyn cicada::store::Store>) -> Result<(), String> {

let system = ActorSystem::builder(store)
    .install(CrashLoopGuard::new(5)?) // CrashLoopGuard::default() is 5; on the node or on one type
    .build();
# Ok(())
# }
```

The message ends `failed` (`onClaim of plugin CrashLoopGuard failed: crash loop: 5 claims without a commit`), is never
retried and shows under `/failures`. Every claim without a commit counts, also a lease lost to a long pause or released
by a stopping node, so keep the limit above what rolling deploys during long turns can cause.

**Dead letters.** A failed message stays in the failures list for the outcome retention. `FailureWatcher` (a job, not
an actor: handlers cannot read `/failures`) copies each failure into the `DeadLetters` singleton
(`actor://cicada/dead-letters`, hosted as `DeadLettersActor`), which keeps them past the retention, at most `capacity`
(oldest dropped first):

```rust
# use std::sync::Arc;
# use std::time::Duration;
use cicada::ActorSystem;
use cicada_extras::{DeadLettersActor, FailureWatcher};
# async fn node(store: Arc<dyn cicada::store::Store>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

let system = ActorSystem::builder(store).host(DeadLettersActor::with_capacity(1_000)? /* , … */).build();
system.start().await?;
let watcher = FailureWatcher::new(system.clone()).prefix("actor://orders/").start(Duration::from_secs(10));
// … on shutdown: watcher.abort();
# Ok(())
# }
```

The watcher reads through the `ActorOperations` trait, which `ActorSystem` implements and, with the `client` feature
of `cicada-extras`, `ActorClient` too, so a watcher can also run in a process of its own. Every node may run one:
each failure is forwarded with an id derived from the failed message's id (the same as Kotlin's), so the store's send
dedup records it once. Act on a letter with a message: `Redrive { failed_message_id }` sends it again (same target,
type, payload, correlation id; once), `Discard { failed_message_id }` forgets it — over HTTP,
`POST /actors/cicada/dead-letters/messages {"type": "Redrive", "payload": {"failedMessageId": "…"}}`.

**Timers and schedules.** A timer is a delayed self-send that is cancelled by making it stale. `Timer`, kept in
state, gets a new generation on every `arm(at)` and `disarm()`; the timeout carries the generation it was armed with,
and only one for which `timer.is_current(generation)` is true counts. The handler still sends:

```rust
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada_extras::Timer;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Checkout { id: String }
# impl ActorRef for Checkout { const TEMPLATE: &'static str = "actor://checkout/{id}"; type Message = CheckoutMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum CheckoutMsg { Begin, PaymentTimeout { generation: u32 } }
# #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
# struct CheckoutState { timer: Timer, expired: bool }
# struct CheckoutActor;
# impl Actor for CheckoutActor {
#     type Ref = Checkout;
#     type State = CheckoutState;
#     type Message = CheckoutMsg;
#     const UNHANDLED: Unhandled = Unhandled::Fail;
#     async fn handle(&self, turn: &mut Turn<Self>, msg: CheckoutMsg) -> HandlerResult {
match msg {
    CheckoutMsg::Begin => {
        let deadline = turn.now() + Duration::from_secs(15 * 60);
        let generation = turn.state_mut().timer.arm(deadline);
        turn.send_self_with(CheckoutMsg::PaymentTimeout { generation }, SendOptions::new().at(deadline))?;
    }
    CheckoutMsg::PaymentTimeout { generation } => {
        if !turn.state().timer.is_current(generation) {
            return Ok(()); // re-armed or disarmed since: stale
        }
        let state = turn.state_mut();
        state.timer.disarm();
        state.expired = true;
    }
}
#         Ok(())
#     }
# }
```

`Schedule` (`Schedule::every(seconds)`, aligned to the epoch, and `Schedule::daily(hour, minute)`, UTC) gives a cron
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

Node settings are `NodeConfig` fields, set with the builder (`ActorSystem::builder(store).lease_ttl(…)`) or all at
once with `.config(NodeConfig { … })`:

| Setting | Default | Meaning |
| --- | --- | --- |
| `node_id` | random | Lease owner, shown in `/messages/{id}` while running and by `/health`. |
| `lease_ttl` | 30 s | A turn's lease; a dead node's turn is re-claimed after it expires. |
| `heartbeat_interval` | `lease_ttl / 3` | Lease renewal; must be in `(0, lease_ttl / 2]`. |
| `poll_interval` | 500 ms | Longest sleep of the dispatch loop (wake-ups usually come sooner: own sends, PostgreSQL `NOTIFY`). |
| `default_max_turn` | 15 min | Turn timeout (`failed("timeout")`) unless the type sets `MAX_TURN` or `ActorType::max_turn`. |
| `concurrency` | 16 | Turns running at once on this node. |
| `shutdown_grace_period` | 30 s | Running turns may finish on stop; then they are cancelled and their leases released. |
| `shutdown_cancel_timeout` | 5 s | How long a cancelled turn may take to end before it is left to its lease. |
| `sweep_interval` | 1 h | Deletion of expired outcomes. |

Handlers run on the tokio runtime that called `start`; there is no separate handler pool (see
[Blocking code](#define-an-actor-type)).

Store settings: outcome **retention 7 days** (`SqliteOptions::retention`, `PostgresStoreConfig::retention`), also the
dedup window. SQLite: `SqliteOptions { clock, retention, busy_timeout: 5 s, readers: 4 }`. PostgreSQL:
`PostgresStoreConfig { schema: "public", max_pool_size: 20, min_idle: 1, connection_timeout: 10 s, socket_timeout:
30 s, channel: "cicada_<schema>", listener_reconnect_delay: 250 ms, transaction_retries: 8 }` (deadlocks and
serialization failures are re-run). [docs/operations.md](docs/operations.md) says how to size them.

## Examples

`examples/` (crate `cicada-examples`) holds ten worked examples, each with its tests under `examples/tests/`, run
with `TestActors` on virtual time:

| Module | Pattern |
| --- | --- |
| `singleton` | A stateless singleton (`actor://maintenance/cleanup`) whose turns never overlap. |
| `timer` | A timer / timeout with a generation number; a stale timeout is ignored. |
| `cron` | A cron job; each tick does the work and arms the next one in the same commit. |
| `reply` | Request and reply; the responder replies to `message().from` with the correlation id. |
| `saga` | An orchestrator saga with compensation, and per-message retry set by the sender. |
| `steps` | Durable steps; every step is its own turn, so a committed step never runs again. |
| `fanout` | Fan-out and fan-in; one child per item, the parent counts the replies. |
| `registry` | A registry actor that keeps the refs of the instances it created. |
| `supervision` | Supervision; failed messages become dead letters that an operator redrives. |
| `sync` | The architecture's example: a paged import, one turn per page, rescheduled hourly. |

Every example has the same shape: a public ref struct per actor type (its fields are the template parameters), a
message enum with `#[serde(tag = "type")]`, a state struct, and an actor struct whose fields are its dependencies
(external services, faked in the tests), with one `impl Actor` whose `handle` matches on the message.

## Sample service

`sample-service` is a runnable node hosting the examples, with the API, the console and the readiness check; see
[sample-service/](sample-service/). It is configured by environment variables, the same as the Kotlin sample:

| Variable | Meaning | Default |
| --- | --- | --- |
| `PORT` | HTTP port | 8080 |
| `NODE_ID` | node id (lease owner, shown by `/health`) | random |
| `HOSTED_TYPES` | `all`, `none`, or a comma-separated list of example groups | `all` |
| `DATABASE_URL` | PostgreSQL, `postgres://user:pass@host:5432/db` or `jdbc:postgresql://…`; selects PostgreSQL | — |
| `DATABASE_USER`, `DATABASE_PASSWORD` | credentials, when not in the URL | — |
| `DATABASE_SCHEMA` | PostgreSQL schema of the store | `public` |
| `DATABASE_POOL_SIZE` | PostgreSQL connection pool size | the store's default (20), or `CONCURRENCY` + 4 when larger |
| `SQLITE_PATH` | SQLite file, used when `DATABASE_URL` is not set | `actors.db` |
| `API_TOKEN` | when set, the HTTP API requires `Authorization: Bearer <token>` | — |
| `LEASE_TTL`, `HEARTBEAT_INTERVAL`, `POLL_INTERVAL`, `SHUTDOWN_GRACE_PERIOD`, `MAX_TURN` | node timings, as short (`5s`, `250ms`) or ISO-8601 (`PT5S`) durations | library defaults |
| `CONCURRENCY` | maximum concurrent turns | 16 |
| `FAILURE_WATCHER_INTERVAL` | poll interval of the dead-letter watcher (group `supervision`) | `10s` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | when set, traces and metrics go there over OTLP (`ActorTelemetry`) | — |
| `CRASH_LOOP_LIMIT` | when set, `CrashLoopGuard` fails a message after this many claims without a commit | — |

The root `docker-compose.yml` starts a PostgreSQL for development and tests
(`postgres://cicada:cicada@localhost:55432/cicada`).

## Building and testing

Requirements: Rust 1.94 or newer, Node.js 22+ with npm for the console (`cicada-ui` builds it in its build script),
and, for the PostgreSQL tests, Docker.

```sh
cargo build --workspace
cargo test --workspace                     # unit, runtime, SQLite, HTTP and README tests
cargo clippy --workspace --all-targets
docker compose up -d postgres              # PostgreSQL on port 55432
CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada-store-postgres
```

Tests that need PostgreSQL are skipped when `CICADA_PG_URL` is unset. Every store backend runs the conformance suite
of `cicada-store-conformance`; `cicada-store-sqlite` also checks its schema against the Kotlin one when the Kotlin
repository is next to this one.

The crates are not published to crates.io (`publish = false`). Use them as git dependencies:

```toml
[dependencies]
cicada = { git = "https://github.com/helico-tech/cicada.rs" }
cicada-axum = { git = "https://github.com/helico-tech/cicada.rs" }
cicada-store-postgres = { git = "https://github.com/helico-tech/cicada.rs" }

[dev-dependencies]
cicada-test = { git = "https://github.com/helico-tech/cicada.rs" }
tokio = { version = "1", features = ["test-util"] }
```

## Interoperability with Kotlin nodes

The Rust port keeps the Kotlin contract (ADR 0001): the store schema (version 6, migrations included), the JSON of
envelopes, states and policies, the HTTP API with its bodies, status codes and problem types, and the console.

- **One database.** `SqliteStore` and `PostgresStore` create and migrate exactly the Kotlin tables, columns, indexes,
  sequences and version records (ADR 0004). Kotlin and Rust nodes can run on one store, and a database written by one
  opens with the other. A node refuses a schema newer than its own, so upgrade both implementations to the same
  schema version.
- **One type, one implementation.** Contract hashes differ between the implementations, because the JSON Schemas
  generated by schemars and by kotlinx-schema differ in detail (integers get a `format`, for example). A template
  registered by a Kotlin node therefore conflicts with the same template from a Rust node, and the second node fails
  to start. Host each actor type in one implementation; types of different templates mix freely.
- **Across implementations.** Any node serves the whole HTTP API for every registered type and sends to it: a Rust
  node can create, send to and read instances of a type that only Kotlin nodes host, and the other way round (only
  the hosting implementation decodes payloads and states). A Rust actor sends to a type hosted by Kotlin nodes with a
  ref struct and message enum of its own that match the Kotlin template and type names (typed sends are resolved
  through the type registry, so the ref need not be hosted), or with `send_raw`.
- **Clients.** `cicada_client::ActorClient` talks to Kotlin nodes, and the Kotlin client to Rust nodes. The dead
  letter id of `FailureWatcher` is the same as Kotlin's, so watchers of both implementations record a failure once
  (the `DeadLetters` type itself is hosted by one of them).

## Decisions and deviations

Decisions of the port, as ADRs in [docs/adr/](docs/adr/):

| ADR | Decision |
| --- | --- |
| [0001](docs/adr/0001-port-keeps-the-contract.md) | The port keeps Cicada's contract: domain language, G1–G6, S1–S10, the HTTP API, the JSON forms and the SQL schema. Only how definitions are written and which libraries are used change. |
| [0002](docs/adr/0002-crate-stack.md) | Crate stack: tokio, axum 0.8, sqlx 0.9 (one store per backend), serde, schemars, chrono, thiserror/anyhow, async-trait, reqwest, rust-embed, tracing and opentelemetry. |
| [0003](docs/adr/0003-actor-definition-api.md) | Actor definitions: a ref struct implementing `ActorRef`, a tagged serde enum, and the `Actor` trait with a required `UNHANDLED` and one `handle`; no proc macro. |
| [0004](docs/adr/0004-shared-store-schema.md) | The SQL schema is shared with Kotlin; contract hashes differ, so one template is hosted by one implementation. |
| [0005](docs/adr/0005-reuse-the-console.md) | The React console is reused unchanged; a fixtures test checks the Rust API against its zod schemas. |

Open questions of the original design, resolved there and kept here:

| Question | Decision |
| --- | --- |
| Crash loops | No claim limit in the core; `attempt` is on the envelope and in `/messages/{id}`. `CrashLoopGuard` is opt-in. |
| Contract changes | Strict contract-hash equality; a node with a different contract for a registered template fails to start. Contracts include the generated schemas, so a schemars upgrade can change hashes. |
| State evolution | No migrations; changes must be additive with defaults (`#[serde(default)]`). |
| Plugin data | `headers` on the envelope; `correlation_id` is kept (the HTTP contract uses it). |
| Send to an absent instance inside a turn | The whole turn fails at commit: outcome `failed` with the reason, no effects. |
| Admin delete | Allowed: `DELETE /actors/{path}`. |
| One-way messaging | Kept; no ask/await. |
| Defaults | Lease 30 s, heartbeat 10 s, poll 500 ms, max turn 15 min, retention 7 days; all configurable. |

How the Rust port differs from the Kotlin implementation:

- **Definitions are checked by the compiler.** One `handle` with an exhaustive `match` replaces `on<T> { }`, and the
  required `const UNHANDLED` replaces the check at node start (ADR 0003). Handlers return
  `Result<(), HandlerError>`: `HandlerError::permanent` / `turn.fail(..)` replace `PermanentFailure` and `fail()`,
  and a panic is caught and treated as a failure (`panic: …`). A custom unhandled policy is `Actor::on_unhandled`,
  returning an `UnhandledDecision`. `Attributes` are keyed by type instead of `AttributeKey<T>`.
- **No application plugin.** There is no `install(Actors)`: the host builds an `ActorSystem`, calls `start` and
  `stop` itself, merges `actor_api()` into its router, and owns the store (no `sqlite(…)` / `postgres(…)` helpers
  that close it). `actor_api()` takes the `ActorSystem` itself, so it cannot be mounted without a node.
- **Handlers run as futures on the tokio runtime.** Kotlin runs them on `Dispatchers.IO`, where blocking calls are
  fine. In Rust a handler that does not yield is cut off only when it returns, and it holds a runtime worker thread
  meanwhile; when every worker is blocked, heartbeats and the dispatch loop stall too. Run blocking work in
  `spawn_blocking`. A handler still running well after its maximum turn or a lost lease is logged as a warning.
- **Validation at construction.** `CrashLoopGuard::new(0)` and `DeadLettersActor::with_capacity(0)` return an `Err`
  instead of failing node start (a plugin has no validation hook). `RetryPolicy::new`, `Backoff::*`, `Keep::last`,
  `Keep::keep_for` and `Schedule::*` return `Result` too.
- **Plain `Timestamp` and `MessageId` in state.** chrono's serde form is the contract's instant format, and both types
  have JSON Schemas, so states and messages hold them without attributes (issue 0701). `RetryPolicy` and `Retention`
  have JSON Schemas too (issue 0200).
- **Telemetry.** The OpenTelemetry SDK for Rust exports every span it drops, so the span of a turn that never commits
  is ended and exported with `cicada.outcome` = `abandoned`; Kotlin never exports it (issue 0600). The W3C
  propagator's empty `tracestate` is left out, so headers equal Kotlin's.
- **Client.** `ActorClient` refuses addresses with a `.` or `..` segment (`ClientError::Invalid`): reqwest's URL
  parser removes such segments even when escaped, and would address another instance. The server and the in-process
  API serve them (issue 0300, won't fix).
- **PostgreSQL settings.** There is no map of extra connection properties: connection parameters go in the URL, and
  `PostgresStore::connect_with(PgConnectOptions, config)` takes sqlx options for the rest (issue 0501).
  `socket_timeout` bounds a whole transaction attempt (the connection is then closed), not every read as pgjdbc's
  does, because sqlx has no per-read timeout.
- **Cancelling `run_due_turns`.** Dropping its future cancels the turns like a node stop after the grace period: the
  worker is stopped first, then the lease is released only when the handler had not ended, and claims that arrive
  after the caller is gone are released.
- **Small differences on the wire and in logs.** Error texts use Rust names (`max_uncommitted_claims must be at least
  1`); a SQLite file of a newer schema is refused with `StoreError::Invalid`. Template parameters are served in
  template order, as in Kotlin.
- **Examples.** Pricing replies with its own `PricingReply` enum instead of reusing the Cart's message classes;
  fallible fakes return a `Result`; `charge_retry()` and `tick_retry()` are functions because the policy constructors
  validate.

Known limitations: no claim limit for crash loops without `CrashLoopGuard`; external side effects are at-least-once
(use `message().id` as idempotency key, also across retries); a commit rejected because of the handler's own sends is
not retryable; the client has no streaming of message status (poll `GET /messages/{id}`).
