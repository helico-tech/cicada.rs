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
        })
        .await?;
    store.close().await; // the store is yours: close it after the node stopped
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
levels deeper, a page of `GET /outcomes` four, `GET /actors/{type}/{key}` a state one, and a `Failed` holds the payload
of the failed message one level down. So no view of a value written from 0.16.0 on is deeper than 69 levels, which
every JSON parser that allows 128 reads, serde_json and `cicada-client` among them. System.Text.Json and Json.NET stop
at 64 by default; each raises it with one setting (`MaxDepth`). A value stored before 0.16.0 can be deeper, and so can
its view.

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
