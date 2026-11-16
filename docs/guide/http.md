# HTTP: serve and call actors

Use `cicada-axum` to mount the API in your service and `cicada-client` to call it.
For a binary that already includes both, use [App](app.md).

## Run it in an axum service

The host owns authentication, signals and shutdown. The routers are ordinary axum routers:

<!-- doctest
# use cicada::{ActorSystem, Store};
# use cicada::store::SqliteOptions;
# async fn example() -> anyhow::Result<()> {
# let store = Store::sqlite("actors.db", SqliteOptions::default()).await?;
# let system = ActorSystem::builder(store.clone()).build();
@@7
# let _ = app;
# Ok(()) }
-->
```rust,no_run
use axum::Router;
use cicada_axum::{actor_api, actor_health};

system.start().await?;
let app = Router::new()
    .merge(actor_api(system.clone()))
    .merge(actor_health(system.clone(), "/ready"));
```

This minimal composition has no authentication. Wrap the API in the host’s authentication layer **before** merging the
public readiness route. `/ready` checks node/store readiness; use a separate process check for liveness.
For complete middleware and lifecycle wiring, see [sample-service/src/main.rs](../../sample-service/src/main.rs).

| Router | Use |
| --- | --- |
| `actor_api(system)` | Full API, including writes and trusted lease operations |
| `actor_api_read(system)` | Read routes only; still requires the host’s access policy |
| `actor_health(system, path)` | Readiness outside the protected API |
| `actor_api_with(system, ApiConfig { .. })` | Customize the body limit (default 1 MiB) |

On SIGTERM/Ctrl-C, stop the node **inside** axum’s shutdown future while HTTP can still answer. Close the store after
the node stops, so SSE responses end before axum waits for open responses. Alternatively bound the HTTP drain as App
does. Closing the store only after axum returns can hang shutdown on a change stream.

## The client

The client accepts the same refs and messages as a local node. This uses the reports actor from
[sync.rs](../../examples/src/sync.rs):

<!-- doctest
@@3
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
@@4
# Ok(()) }
-->
```rust,no_run
use cicada_client::ActorClient;
use cicada_examples::sync::{Reports, ReportsMsg, ReportsState};

let client = ActorClient::new("http://127.0.0.1:8080");
let reports = Reports { tenant: "acme".into() };
client.create(&reports, &ReportsState::default()).await?;
client.send(&reports, ReportsMsg::SyncFinished { imported: 5 }).await?;
```

Use `ActorClient::with_client` with a configured `reqwest::Client` for authentication headers.
Expected alternatives are values: missing actors are `None`, duplicate sends set `receipt.duplicate`.
Other HTTP problems return `ClientError::Api`.

To choose placement at creation, use the same options as the core:

```rust,no_run
use cicada::CreateOptions;
use cicada::store::NodeId;
use cicada_client::ActorClient;
use cicada_examples::sync::{Reports, ReportsState};

async fn create_on_host(client: &ActorClient, reports: &Reports) -> anyhow::Result<()> {
    let options = CreateOptions::new().pinned_to(NodeId::new("host-a")?);
    client.create_with(reports, &ReportsState::default(), options).await?;
    Ok(())
}
```

`create_actor_with` is the raw-address form. An existing actor keeps its pin; the create response has only `address`
and `created`, so read the actor to see its actual placement. [Core placement](core.md#pin-an-actor-to-an-executor)
explains stable ids, waiting work and moving an actor.

Sends retry I/O failures and 503 responses with the **same id** (3 attempts by default). This is distinct from
[handler retry](core.md#retry). Lease operations have their own rules below.

## HTTP API

The [OpenAPI document](../openapi.json) contains the exact request and response schemas.
`GET /openapi.json` serves the live copy. Payload/state schemas for each actor type come from `GET /types/{name}`.
Use `{type}/{key}` from the canonical address, preserving its percent-encoded key.

| Task | Routes |
| --- | --- |
| Create, inspect, delete | `PUT`, `GET`, `DELETE /actors/{type}/{key}` |
| List addresses or state | `GET /actors`, `GET /actors/states` |
| List direct children | `GET /actors/{type}/{key}/children` |
| Send | `POST /actors/{type}/{key}/messages` |
| Inspect/control mailbox | `GET /actors/{type}/{key}/mailbox`; `POST …/mailbox/pause`, `resume`, `purge` |
| Inspect/cancel running turn | `GET /actors/{type}/{key}/turn`; `POST …/turn/cancel` |
| List messages or outcomes | `GET /messages`, `GET /outcomes`, `GET /turns` |
| Inspect message/cause chain | `GET /messages/{id}`, `GET /messages/{id}/caused` |
| Control a message | `POST /messages/{id}/cancel`, `retry`, `deliver-now` |
| Bulk cancel/retry | `POST /messages/bulk-cancel`, `POST /messages/bulk-retry` |
| Inspect nodes/counts | `GET /nodes`, `GET /nodes/{node}`, `GET /stats` |
| Inspect/register/remove types | `GET /types`; `GET`, `PUT`, `DELETE /types/{name}` |
| Manage watches | `GET`, `PUT`, `DELETE /watches` |
| Follow state changes | `GET /changes` (SSE) |
| Execute remote turns | `POST /leases`, `/leases/renew`, `/leases/release`, `/leases/complete` |
| Readiness | `GET /health` |

A create body is `{"state":{}}` (state is required). Add `"pinnedTo":"host-a"` to pin a new actor;
absent or null means unpinned. A send body is `{"type":"Run","payload":{}}`.
Sends can also carry `id`, `delay` **or** `deliverAt`, `correlationId`, `headers`, `retry` and `retention`.
Creating an existing actor leaves it untouched (200); a new create is 201. New sends return 202; duplicates return 200.
Sending never creates an actor.

### Errors and limits

Errors are RFC 9457 `application/problem+json`, with `type: urn:cicada:problem:<slug>`.

| Status | Meaning |
| --- | --- |
| 400 | Invalid envelope, query, path or cursor; unknown request keys; invalid policy; NUL or excessive depth |
| 404 | No such actor, message, type, node or running turn |
| 409 | An id belongs to another target/type, incompatible contract, or operation cannot apply now |
| 413 | Body exceeds the configured byte limit |
| 422 | Schema/decoder refused the payload, state, address or contract |
| 503 | Store unavailable; operations may be retried according to their idempotency rules |
| 500 | Unexpected server failure, including unreadable stored data |
| 401/403 | Host authentication/authorization response |

Ids and message type names are at most 200 characters; messages have at most 64 headers.
`pinnedTo` must be a non-blank string of at most 200 characters with no NUL, or null; invalid pins are `400`.
Payload/state depth is at most 64; registered schemas at most 120. HTTP views can be up to 69 levels deep, so configure
client JSON parsers for at least 128. Older stored data can exceed this.

Times are ISO-8601 UTC; durations are ISO-8601 strings such as `PT5S`. Listings use keyset cursors (`next`) and a
1–1000 `limit` (default 100). Keep the filter while paging. Pages do not share a snapshot; a rescheduled message can
appear twice in a mailbox traversal, so deduplicate by id. Bulk calls affect at most 1000 messages and report `more`.

Cancelling a pending message is distinct from cancelling a running turn. Running cancellation takes up to a heartbeat,
only wins before the handler ends, and produces no effects. A retry of a failed/cancelled message is a **new id**;
it can repeat external work. A purge leaves the running message; a delete removes the entire actor tree.
See [operations](../operations.md#on-call) before using destructive controls.

### Register a type over HTTP

`PUT /types/{name}` accepts `{state, messages}` schemas only. The server computes the hash:
a new or equal registration returns 200 with the first stored descriptions; a different hash returns 409 and replaces
nothing. Use the contract generated by `contract_of::<A>()`, rather than a second handwritten definition.

Registration runs no handler. Creates and sends wait until a host of the type starts. Schema structure/depth is checked,
but schemas are not compiled at registration. This is a trusted write, available only on the full router.

### Run a worker over HTTP

The easiest path is [App’s `worker --server`](app.md#a-worker-over-http).
For an embedded worker, `ActorClient` implements `TurnSource`; the complete tested implementation is
[remote_worker.rs](../../examples/src/remote_worker.rs).

<!-- doctest
# use cicada_examples::singleton::TempFiles;
# async fn example<F: TempFiles>(files: F) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
@@5
# Ok(()) }
-->
```rust,no_run
use cicada_examples::remote_worker::start_cleanup_worker;

let worker = start_cleanup_worker("https://actors.internal", "token", files).await?;
tokio::signal::ctrl_c().await?;
worker.stop().await;
```

Set a stable, unique `WorkerConfig::worker_id` for pinned work. Leases use that id, never the server's: a worker
runs actors pinned to it and unpinned actors of its types. `Turn::node_id()` returns the worker's id. Returned creates
in a `/leases/complete` report accept optional `pinnedTo` with the same bounds and absent/null rules as HTTP creates.

Upgrade all servers before sending `pinnedTo`; 0.19 servers reject it as an unknown field. A 0.19 worker over HTTP
can still use a 0.20 server, which enforces pins, but cannot create pinned actors. This does not permit mixing old
and new [direct-store runtimes](../how-to/upgrading.md#upgrade-to-0200).

When constructing your own HTTP client, disable redirects and transport retry layers. Each lease call must issue
one request; the worker controls completion retries with the same report. `turn_timeout` bounds all four calls through
the response body (10 s default), subject to any shorter client timeout.

| Response | Turn-source classification |
| --- | --- |
| 400, 413, 422 | Invalid |
| 408, 429, 502, 503, 504, transport/timeout | Unavailable; a mutation may have applied |
| Other error status, malformed complete body, unknown result | Fatal |

Lost leases, contract mismatches and rejected commits are **200 tagged results**, not HTTP error statuses.
A completion whose answer was lost may have applied; `leaseLost` after a repeat does not settle it.
After `rejected`, submit only the returned failure. Never rerun a finished handler or release its lease.

Protect the type-registration and all lease routes with authentication. Epochs are fencing tokens, not secrets.
Workers need type reads as well as writes. No sticky session is required, but every selected server must support the
protocol (0.16.16 or newer). Size proxy/server body limits for a whole completion report, not one outgoing message:
repeated 413 responses can exhaust the [claim limit](core.md#claim-limit).

For a worker implementation in another language, test against the portable fixtures shipped from 0.18.12 onward.
Download the archive matching your server version; for 0.20.0:

```sh
gh release download v0.20.0 --repo helico-tech/cicada.rs --pattern 'cicada-fixtures-*.tar.gz'
tar -xzf cicada-fixtures-v0.20.0.tar.gz
```

`cicada-fixtures/SOURCE.json` identifies the repository, tag, commit and version. The four files declare `format: 1`:

| File | Check |
| --- | --- |
| [canonical.json](../../fixtures/canonical.json) | Canonical text and hashes, including parser refusals and the tested dependency baseline |
| [contracts.json](../../fixtures/contracts.json) | Contract hashes after removing schema descriptions |
| [leases.json](../../fixtures/leases.json) | Bodies for the four lease operations: valid, accepted-and-normalized, and refused at decode or conversion |
| [status.json](../../fixtures/status.json) | Success statuses, error classification and type-registration exceptions |

JSON inputs and bodies are strings inside the files. Parse those strings with a parser that distinguishes integers
from floats. Match canonical text and hashes byte for byte using the [canonical rules](../architecture.md#http-contract).
Compare lease bodies by meaning: Rust's member order and numeric spelling are examples. Successful decoding does
not establish that a server admits a lease or applies a report's effects.

The registration exceptions include `422 invalid-contract` and `409 contract-conflict` with one `GET /types/{name}`;
a `404 no-such-type` on that follow-up is Unavailable. Case names are stable; format 1 can gain cases, while changed
field meanings need a new format. Finite fixtures do not prove compatibility for every number or dependency graph.

### List actors with their state

`ActorClient::list_actors_with_state` / `GET /actors/states` returns full actor snapshots in one store read per page.
Use this for dashboards instead of listing addresses then fetching each actor. Keep pages small for large states;
one unreadable row fails the page. Each row, like `GET /actors/{type}/{key}`, includes `pinnedTo` (`null` when
unpinned). New clients read an older server's missing field as `None`.
See [core paging semantics](core.md#list-actors-with-their-state).

### Follow changes over HTTP

Subscribe to `GET /changes?prefix=…`, then reload the prefix on `resync` and reread each address on `changed`:

```text
event: resync
data: {}

event: changed
data: {"address":"orders/a"}
```

The first event is `resync`. Reconnect with a new full read; there are no event ids, history or replay.
A keep-alive comment after 15 s of silence is not a readiness signal. Subscribe before listing, process queued hints
after the read, and replace your view on every resync. The [core guide](core.md#follow-changes) explains coverage.

Disable proxy buffering/compression for this route and allow timeouts longer than 15 s. Browser `EventSource` cannot
set an Authorization header: use the host’s session cookies and appropriate credentialed CORS.
`ActorClient` has no SSE method. A node stop leaves streams open; store close ends them. A request after store close
is the special case of an empty 200 response.

### The schema check

Every node checks HTTP creates/sends against the registered schema, even without the actor’s handler.
A hosting node also decodes with serde. Limits are checked before deduplication; an accepted duplicate is answered before
schema validation. Code sends skip JSON Schema validation; serde remains authoritative at the turn.

Test representative client data with `cicada_axum::assert_message_sample` and `assert_state_sample`.
Test both the serializer’s output and alternate forms clients use (for example aliases).
No `format` is asserted and no external reference is fetched. If a schema cannot compile, it is **not enforced**:
the node warns once and sample tests fail. Correcting a schema can change the contract; follow the
[contract deployment procedure](../operations.md#changing-a-contract).
