# HTTP and the client: `cicada-axum` and `cicada-client`

Layer 1 of Cicada (ADR 0018): the HTTP API of a node, with the schema check at the door, and the typed client. Both
use only the public API of [the core](core.md), whose guide covers the node and its store. [The app](app.md) serves
this API for a host that does not need more.

| Crate | Contents |
| --- | --- |
| `cicada-axum` | `actor_api()` (the whole API), `actor_api_read()` (only what reads) and `actor_health()` (a public readiness check) as axum routers; the check of a send and a create against the registered JSON Schemas, and the sample checks `assert_message_sample` and `assert_state_sample`. |
| `cicada-client` | `ActorClient`: typed HTTP client (reqwest) for remote callers, idempotent sends. |

The Rust blocks of this guide are doctests of the `examples` crate (`cargo test -p cicada-examples --doc`).

## Run it in an axum service

A host that needs more than `App` uses the layers directly. It owns the store and the node's life cycle; the HTTP API
is a `Router` it mounts with its own authentication (`SyncActor` and `ReportsActor` are from
[examples/src/sync.rs](../../examples/src/sync.rs)):

<!-- doctest
# use cicada_examples::sync::{Page, ReportsActor, Sync, SyncActor, SyncApi, SyncMsg};
# struct HttpSyncApi;
# impl SyncApi for HttpSyncApi {
#     async fn fetch(&self, _: &str, _: Option<&str>, _: &str) -> Page { Page { size: 0, next: None } }
# }
@@54
# async fn start_sync(State(system): State<ActorSystem>, Path(tenant): Path<String>) -> StatusCode {
#     match system.send(&Sync { tenant }, SyncMsg::Run).await {
#         Ok(SendResult::Accepted(_)) => StatusCode::ACCEPTED,
#         Ok(SendResult::NoSuchInstance) => StatusCode::NOT_FOUND,
#         Ok(_) | Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
#     }
# }
#
# /// The host's authentication: the API has none of its own.
# async fn bearer(State(token): State<Arc<str>>, req: Request, next: Next) -> Response {
#     let expected = format!("Bearer {token}");
#     match req.headers().get(header::AUTHORIZATION) {
#         Some(value) if value.as_bytes() == expected.as_bytes() => next.run(req).await,
#         _ => StatusCode::UNAUTHORIZED.into_response(),
#     }
# }
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
            store.close().await; // the store is yours: close it after the node; it also ends open GET /changes streams
        })
        .await?;
    Ok(())
}

// `start_sync` and `bearer` as in the app guide, Run it as a binary.

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("a SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}
```

- `system.start()`, `system.stop()` and the store are those of the core: see
  [Open a store and run a node](core.md#open-a-store-and-run-a-node).
- A node hosting no types still serves the whole API (create, send, query) for every registered type.
- The store is closed inside the shutdown future, after the node stopped and before axum waits for open requests:
  axum's graceful shutdown waits for every open response, and a [change stream](#follow-changes-over-http) stays open
  until its store closes. A host that closes the store later bounds that wait in another way, as `cicada-app` does
  with its five seconds.
- `actor_api(system)` and `actor_health(system, path)` are plain routers with no fallback: `merge` them or `nest`
  them under a prefix, and wrap them in your own layers. They need nothing installed on the app: they read and write
  JSON themselves with the wire types of `cicada::api`. `actor_api_with(system, ApiConfig { max_request_body_bytes })`
  changes the body limit.

## The client

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
| `GET /actors/states?prefix=&cursor=&limit=` — the actors of `GET /actors`, each as `GET /actors/{type}/{key}` shows it, with the same pages and cursors; one read of the store per page (see [List actors with their state](#list-actors-with-their-state)) | 200 | 500 a stored row that no longer decodes fails the whole page |
| `GET /changes?prefix=` — hints that actors under the prefix may have changed, as Server-Sent Events (see [Follow changes over HTTP](#follow-changes-over-http)) | 200 `text/event-stream`, also after the store closed (an empty body) | |
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
| `POST /messages/{id}/retry` — send a `failed` or `cancelled` message again: a new message with a new id, same target, type, payload, correlation id, headers, retry and retention, `causedBy` the original | 202 `{id, deliverAt}` | 404 unknown, past retention or removed by its retention policy; 404 `no-such-instance` when the target is gone; 409 `message-not-retryable` for a pending, running, done or purged message, and for one whose payload is nested deeper than 64 levels (stored before 0.16.0, or a `Failed` about a payload at the limit) |
| `POST /messages/{id}/deliver-now` — make a pending, not running message due at once, at a new place in delivery order | 200 message | 404; 409 running or already finished |
| `POST /messages/bulk-cancel` body `{ids? or prefix?, messageType?}` — cancel by ids (at most 1000) or by filter, at most 1000 per call; skips what cannot be cancelled | 200 `{affected, more}` | 400 not exactly one of ids or a filter, or more than 1000 ids |
| `POST /messages/bulk-retry` body `{ids? or prefix?, messageType?}` — retry by ids or by filter (over `failed` outcomes), at most 1000 per call; it skips, without counting, what cannot be retried, also a payload nested too deep, and a filter skips messages already retried | 200 `{affected, more}` | 400 |
| `GET /messages/{id}/caused?cursor=&limit=` — the messages that the turn which handled `{id}` sent, one level down: pending ones and retained outcomes, each like `GET /messages/{id}`, in message-id order | 200 | 404 unknown or past retention |
| `GET /outcomes?status=&prefix=&since=&cursor=&limit=` — finished messages with envelope and error; `status` is `done`, `failed`, `cancelled` or `purged` (absent: all), oldest first | 200 | 400 unknown status |
| `GET /turns?node=&prefix=&cursor=&limit=` — the turns running now, by address: actor, message, node, lease expiry, `cancelRequested` | 200 | |
| `GET /nodes` — every node: id, hosted types, started and last seen, status (`alive`, `suspect`, `dead`), `runningTurns` (count) | 200 | |
| `GET /nodes/{node}` — one node, and the list of its running `turns` | 200 | 404 `no-such-node` |
| `GET /stats?prefix=` — `{actors, pending, oldestDue}`: the number of actors and pending messages under the prefix, and the delivery time of the oldest message that is due (null when none is). It reads every row under the prefix | 200 | |
| `GET /types` — type names, contract hashes, state and message JSON Schemas | 200 | |
| `GET /types/{name}` — one type | 200 | 404 |
| `PUT /types/{name}` body `{state, messages}` — register a contract without hosting the type; the server computes the hash, the first registration wins (see [Register a type over HTTP](#register-a-type-over-http)) | 200 the stored type, new or equal | 400 a malformed body, a missing field, any other field (`name`, `hash`, …) or a path that is no type name; 409 `contract-conflict`: the name holds another hash; 422 `invalid-contract`: a schema that is not an object or a boolean, has a NUL or is nested deeper than 120 levels, or a message name beyond the send limits |
| `DELETE /types/{name}` — removes the type's registration (a node that hosts the type registers it again at its next start) | 204 | 404; 409 actors of the type exist |
| `GET /watches?watcher=&target=&cursor=&limit=` — stored watches `{target, watcher}` in the order of target and watcher; `target` narrows to the watches stored with exactly that target | 200 | 400 a watcher or target that does not parse |
| `PUT /watches` body `{target, watcher}` — set a watch; the target is an address, a type prefix or `*`, and need not exist | 201 set; 200 set already | 400 a target that is no target; 404 `no-such-type` or `no-such-instance` for the watcher; 422 `not-a-watcher` when its type does not declare `Failed` and `Stopped` |
| `DELETE /watches?target=&watcher=` — remove a watch | 204 | 400; 404 `no-such-watch` |
| `GET /openapi.json` — the OpenAPI 3.1 document of this router | 200 | |
| `GET /health` — node running and store reachable | 200 | 503 |

`actor_api(system)` serves all of this. `actor_api_read(system)` serves only what reads: it has no route that changes
anything, so it can be exposed more widely. Authentication stays the host's for both.

Clients can be generated from the OpenAPI document. A copy is in [docs/openapi.json](../openapi.json), and a test fails
when it no longer matches the routes (`CICADA_UPDATE_OPENAPI=true` updates it). Payloads and states are free-form JSON in
the document; the schemas of a type come from `GET /types/{name}`. The document is OpenAPI 3.1, which not every generator
reads yet (Progenitor reads 3.0 only), so `cicada-client` stays hand-written.

On every endpoint, additionally:

- `400` — malformed JSON, path or query; an unknown key in a request envelope (`{"paylaod": …}`, also inside `retry`
  and `retention`); both `deliverAt` and `delay`; a delivery time beyond year 9999; envelope limits (id/type ≤ 200
  chars, ≤ 64 headers, no NUL); a payload or a state with a NUL character or nested deeper than 64 levels (`The
  payload is nested deeper than 64 levels`); an invalid retry policy; `limit` outside 1..1000; a bad cursor. Within
  these limits, payloads and states are checked against their schemas (see [The schema check](#the-schema-check)).
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

**How deep a value and its view can be.** A payload and a state hold at most 64 levels of arrays and objects (ADR 0029,
[How deep a value can be](core.md#how-deep-a-value-can-be)). A view wraps them: `GET /messages/{id}` puts a payload two
levels deeper, a page of `GET /outcomes` four, `GET /actors/{type}/{key}` a state one, a page of `GET /actors/states`
a state three (67 levels for a state at the limit), and a `Failed` holds the payload
of the failed message one level down. So no view of a value written from 0.16.0 on is deeper than 69 levels, which
every JSON parser that allows 128 reads, serde_json and `cicada-client` among them. System.Text.Json and Json.NET stop
at 64 by default; each raises it with one setting (`MaxDepth`). A value stored before 0.16.0 can be deeper, and so can
its view: one such state can make a whole page of `GET /actors/states` too deep for a client's parser.

### Register a type over HTTP

`PUT /types/{name}` registers a contract on a server that does not host the type (ADR 0036; in code
`ActorSystem::register_contract`, [Register a contract without hosting](core.md#register-a-contract-without-hosting)).
The body holds only the schemas; the name comes from the path, and the server computes the hash:

```sh
curl -X PUT localhost:8080/types/counter -H "Authorization: Bearer $API_TOKEN" \
     -d '{"state":{"type":"object"},"messages":{"Add":{"type":"object"}}}'
# 200 {"name":"counter","hash":"sha256:…","state":{"type":"object"},"messages":{"Add":{"type":"object"}}}
```

A worker registers the contract its actor type generates, with the typed client; this block registers the reports
type of [examples/src/sync.rs](../../examples/src/sync.rs):

<!-- doctest
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
@@7
# Ok(()) }
-->
```rust,no_run
use cicada::actor::contract_of;
use cicada_client::ActorClient;
use cicada_examples::sync::ReportsActor;

let client = ActorClient::new("http://localhost:8080");
let stored = client.register_type(&contract_of::<ReportsActor>()?).await?; // only the schemas cross the wire
println!("{} {}", stored.name, stored.hash);
```

- **Retry freely.** A new and an equal registration both answer `200` with the stored type, which keeps the
  descriptions of the first registration. A lost response is retried with the same body.
- **No replacement.** Another hash for the name is `409 contract-conflict`, whose detail names the type and both
  hashes; the client returns it as `ClientError::Api`. Delete the unused registration (`DELETE /types/{name}`) and
  register again to change it.
- **Before a worker runs.** Creates and sends are checked against the registered schemas at once, on every node, and
  the messages wait until a node that hosts the type claims them. Registration runs nothing.
- **Limits.** The schema checks are those of the core (a schema at most 120 levels deep, else `422 invalid-contract`);
  the body limit and the JSON parser of every request still apply. The schemas are not compiled, so an unusable one is
  accepted and the door warns about it later ([The schema check](#the-schema-check)).
- **Who may register.** It is a write route: `actor_api` serves it, behind the host's authentication, and
  `actor_api_read` answers `405`. It is not an identity or an authorization of a worker.

### List actors with their state

`GET /actors/states` and `ActorClient::list_actors_with_state` list actors with their state (ADR 0033): each row is
the view of `GET /actors/{type}/{key}` (address, type, state, version, createdAt, paused, parent), in the pages of
`GET /actors`, with the same `prefix`, `cursor` and `limit` (1 to 1000, 100 by default). Both routers serve it.

<!-- doctest
# async fn board(client: &cicada_client::ActorClient) -> Result<(), cicada_client::ClientError> {
@@9
# Ok(())
# }
-->
```rust,no_run
let mut cursor: Option<String> = None;
loop {
    let page = client.list_actors_with_state("factory.ticket/", cursor.as_deref(), Some(100)).await?;
    for actor in &page.items {
        println!("{} v{}: {}", actor.address, actor.version, actor.state);
    }
    let Some(next) = page.next else { break };
    cursor = Some(next);
}
```

A page is one read of the store, so its rows share one committed snapshot; there is no snapshot across pages (a later
page shows later commits, misses an actor created before its cursor and leaves out a deleted one), and a version
counts the state writes of one life of an actor. Large states make large pages: pick a smaller `limit` for them. One
row that no longer decodes fails the whole page with `500`. The core guide has the details:
[List actors with their state](core.md#list-actors-with-their-state). The route is new in 0.16.9: upgrade the server
before a client uses it. Against an older server the method returns that server's error and does not fall back to
reading each actor.

### Follow changes over HTTP

`GET /changes?prefix=` streams the change hints of the core ([Follow changes](core.md#follow-changes), ADR 0034) as
Server-Sent Events, for a page that shows many actors (ADR 0035). Both routers serve it. It is only a transport: it
covers what the core's feed covers, with the same best effort, and adds no state, history, order or delivery promise.

```text
event: resync
data: {}

event: changed
data: {"address":"order/42"}

: keep-alive

```

- **Events.** `changed` carries the canonical address of an actor that may have changed: read it with
  `GET /actors/{address}` (`404` means it is gone). `resync` says that hints may have been missed: list the prefix
  again with `GET /actors/states`. The first event is a `resync`, once the store's source of hints is live. Every
  event ends in a blank line; ignore an event name you do not know.
- **No replay.** No `id` or `retry` field; `Last-Event-ID` is ignored, and every request, a reconnect too, is a new
  subscription that starts with a `resync`.
- **The query.** `prefix` is the literal prefix of the listings (`order/`, `user/jan%40`), decoded once; absent or
  empty means every actor, the first one counts, and other keys (`cursor`, `limit`) are ignored.
- **Readiness.** The `200` comes at once, without a store query. A `: keep-alive` comment follows each 15 seconds
  without output. Comments can come before the first `resync` (on PostgreSQL while its listener is down) and do not
  mean the stream is live; the first `resync` may wait as long as the database is away, so a page that cares sets its
  own deadline.
- **How long it lives.** Closing the store ends the body without a final event, and a request after the close gets a
  `200` with an empty body, not `503`. Stopping the node does not end it. `cicada-app`'s `serve` stops the node, gives
  open requests 5 seconds and then cuts them with a transport error, so an open dashboard can use those 5 seconds of
  the stop. A host that mounts the routers itself owns that: axum's graceful shutdown waits for open streams until the
  store is closed (see [Run it in an axum service](#run-it-in-an-axum-service)) or the host cancels them.
- **A slow reader.** Each response holds one subscription and nothing more: a body that is not read falls behind,
  and the core's lag turns into a `resync` once it is read again. A dropped connection frees the subscription.
- **Authentication and CORS.** The host's layers wrap the route like any other. The browser's `EventSource` cannot
  set an `Authorization` header: use a session cookie on the same origin, or, from another origin,
  `new EventSource(url, { withCredentials: true })` with CORS that allows that origin and credentials. A `401` or
  `403` closes the source for good.
- **Proxies.** The response says `Cache-Control: no-cache` and `X-Accel-Buffering: no` (nginx's hint not to buffer),
  but not every proxy obeys it: turn off buffering and compression for the route, and allow responses that stay open
  and idle for more than 15 seconds. Over HTTP/1 a browser opens only a few connections per origin, so keep one
  stream per page.
- **Clients.** `cicada-client` has no method for the stream: a browser uses `EventSource`, Rust in the same process
  uses `ActorSystem::changes`, and a remote Rust client brings an SSE reader of its own.

A page keeps one `EventSource` and one worker that does one read at a time, not a fetch per event. It waits for the
first `resync`, lists the prefix into a fresh map and shows the map only when the listing is complete and no newer
`resync` came meanwhile; then it reads the addresses that changed during the listing. It keeps at most 1000 of them
and lists again when there would be more. A read that fails keeps its address, or asks for a new listing, and tries
again after a pause, with one timer at most. The state of the connection is kept apart from the reads, so a read that
succeeds after the source closed for good does not show the view as live again. After each `await` it checks that the
view was not disposed, so a result that arrived just before disposal changes and shows nothing. This JavaScript is an illustration: no
test runs it. The same protocol (hints during paging, a newer `resync`, the bound, the retries, the state of the
connection and disposal) is tested in Rust against a real stream in
[crates/cicada-axum/tests/changes.rs](../../crates/cicada-axum/tests/changes.rs).

```js
// A live view of the actors under `prefix`; returns a function that stops it.
function follow(prefix, render) {
  const MAX_DIRTY = 1000, RETRY_MS = 2000;
  let view = new Map();     // canonical address -> the actor as GET /actors/{address} shows it
  const dirty = new Set();  // addresses to read again
  let generation = 0;       // moves on with each resync
  let needsListing = false, working = false, retry = null;
  let connection = "connecting"; // the stream's state, apart from the reads: "live", "reconnecting" or "stopped"
  const abort = new AbortController();
  // Same origin: the session cookie goes along. URLSearchParams encodes the prefix once.
  const source = new EventSource(`/changes?${new URLSearchParams({ prefix })}`);

  function show() { render(view, connection); }
  function relist() { generation += 1; dirty.clear(); needsListing = true; }
  source.addEventListener("resync", () => { connection = "live"; relist(); kick(); });
  source.addEventListener("changed", (event) => {
    const { address } = JSON.parse(event.data); // canonical and already encoded: use it as it is
    if (!dirty.has(address) && dirty.size >= MAX_DIRTY) relist(); else dirty.add(address);
    kick();
  });
  source.onerror = () => {
    // CONNECTING: the browser reconnects by itself, and the new stream starts with a resync.
    // CLOSED: no reconnect; the page keeps showing that, whatever a read in flight brings later.
    connection = source.readyState === EventSource.CLOSED ? "stopped" : "reconnecting";
    show();
  };

  function kick() { if (!working && retry === null) work(); }
  function later() { retry = setTimeout(() => { retry = null; kick(); }, RETRY_MS); }
  // Called after every await: the page may have been disposed while a promise settled, and aborting cannot take back
  // a result that has already arrived, so nothing is applied or shown after disposal.
  function stillLive() { if (abort.signal.aborted) throw new DOMException("disposed", "AbortError"); }

  async function work() {
    working = true;
    try {
      while (!abort.signal.aborted) {
        if (needsListing) {
          needsListing = false;
          let listed;
          try { listed = await list(generation); stillLive(); } catch (error) { needsListing = true; throw error; }
          if (listed !== null) { view = listed; show(); } // null: a newer resync, list again
        } else if (dirty.size > 0) {
          const address = dirty.values().next().value;
          dirty.delete(address);
          try { await read(address); stillLive(); } catch (error) {
            if (dirty.size >= MAX_DIRTY) relist(); else dirty.add(address); // never lose it
            throw error;
          }
          show();
        } else return;
      }
    } catch (error) {
      if (!abort.signal.aborted) later();
    } finally {
      working = false;
    }
  }

  async function list(gen) {
    const fresh = new Map();
    let cursor = null;
    do {
      const query = new URLSearchParams({ prefix, limit: "100" });
      if (cursor !== null) query.set("cursor", cursor);
      const page = await getJson(`/actors/states?${query}`);
      stillLive();
      if (gen !== generation) return null; // a resync came: this listing may miss what it reported
      for (const actor of page.items) fresh.set(actor.address, actor);
      cursor = page.next;
    } while (cursor !== null);
    return fresh;
  }

  async function read(address) {
    const response = await fetch(`/actors/${address}`, { signal: abort.signal }); // not encoded again
    stillLive();
    if (response.status === 404) { view.delete(address); return; }
    if (!response.ok) throw new Error(`GET /actors/${address}: ${response.status}`);
    const actor = await response.json();
    stillLive();
    view.set(address, actor);
  }

  async function getJson(url) {
    const response = await fetch(url, { signal: abort.signal });
    stillLive();
    if (!response.ok) throw new Error(`GET ${url}: ${response.status}`);
    const body = await response.json();
    stillLive();
    return body;
  }

  // Disposal: the source closes, a fetch in flight is aborted, a result that arrived meanwhile is dropped at the next
  // stillLive(), and the timer is cleared.
  return () => { abort.abort(); source.close(); clearTimeout(retry); retry = null; };
}
```

An address is already encoded: `user/jan%40example.com` is read at `/actors/user/jan%40example.com`, and a key with a
slash, `files/a%2Fb`, at `/actors/files/a%2Fb`; encoding it again (`%2540`) names another actor. Query values are
encoded once, by `URLSearchParams`: the prefix `user/jan%40` goes on the wire as `prefix=user%2Fjan%2540`. After a
`CLOSED` source (a `401`, a `403`, a refused connection) the page shows that it stopped; it may open a new
`EventSource` after the user signs in again, with a growing pause, and that stream starts with a `resync`. Each
`resync` rereads the whole prefix, so many of them cost many rows; there is no snapshot across pages and no history to
catch up from. A page that must see what the feed does not cover (a separately opened SQLite file, PostgreSQL nodes
before 0.16.10, SQL by hand) keeps a periodic full read.

### The schema check

A send and a create over HTTP are checked against the registered JSON Schema of their type, on every node, whether or
not the node hosts the type (ADR 0026): the payload against the schema of its message type, the state of a create
against the state schema. One that does not match is refused with `422 invalid-payload` or `422 invalid-state`, and
nothing is stored. A node that hosts the type also decodes a value that the schema accepts, with serde, with the same
answer. The detail says which check refused it. A refusal by the schema names no place and quotes nothing of the
value, and is the same on every node: `Payload of 'Add' does not match the registered schema; GET /types/counter has
it`. The check asks the validator only whether the value matches, as finding a place can cost time and memory without
bound for a type with several shapes, and after a refusal nothing else runs, not even the decoder of a hosting node, so
a refusal by the schema costs time and memory in proportion to the size of the request. The sample checks name the
first place that does not match. A detail is cut at 2,000 characters with the mark ` … (cut)`.

The order of the checks stays: the limits of the envelope and of the value come first, so a payload nested deeper
than 64 levels is `400` also when it repeats the id of a message that a node of 0.15.0 accepted; then a send that
repeats the id of a message that is pending, or whose outcome is still shown, is answered as a repeat (`200`) before
it is checked; an unknown message type is `422 unknown-message-type`.
In a create the schema comes before what only a hosting node checks, so a state that does not match gets the same
answer on every node; a name that the ref does not accept (`422 invalid-address`) is judged after it, by a hosting
node.

What the door does not check:

- **Formats.** `format` (`date-time`, `email`, …) is a note, as draft 2020-12 has it: a text that is no date passes the
  door, and the message fails in its turn.
- **A schema the validator cannot use.** A schema that does not compile (a `$ref` outside the document, a pattern its
  regular expressions do not know) is not enforced; a warning names it once. The check loads nothing from outside the
  schema: no file, no network.
- **JSON that code sends.** `turn.send_raw` and `ActorSystem::send_message` are not checked against the schema; only a
  hosting node's decoder judges them, as before, after the limits of a value. Typed sends are checked by the
  compiler.

The schema and serde come from the same type, but can disagree. A payload that serde accepts and the schema refuses is
refused at the door: a field sent under its `#[serde(alias)]`, a form that only a hand-written `Deserialize` takes,
a payload that repeats `"type"` for a family that denies unknown fields (the decoder drops it, the schema does not
allow it), or what the type itself writes when a field has `#[serde(with)]` and no `#[schemars(with)]`. In the last
case every send of the typed client is refused, there is no switch, and the way out is a right schema, a contract
change (see [Changing a contract](../operations.md#changing-a-contract)). The other way round, a payload that only
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
