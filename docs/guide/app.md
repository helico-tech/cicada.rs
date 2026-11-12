# The app: `cicada-app`

Layer 2 of Cicada (ADR 0018, ADR 0027): `App` turns a set of actor types into a binary with a command line. It only
composes [the core](core.md) and [the HTTP layer](http.md); a host that needs more uses those directly.

| Crate | Contents |
| --- | --- |
| `cicada-app` | `App`: a set of actor types as a binary with `serve` (the node and the HTTP API), `worker` (the node only) and the operator commands, configured by flags and `CICADA_…` variables; see [Run it as a binary](#run-it-as-a-binary). It builds the binary `cicada`: kubectl-style commands and a k9s-style terminal UI over the HTTP API; see [Command line and terminal UI](#command-line-and-terminal-ui). The stores are its features `sqlite` and `postgres`, the terminal UI its feature `tui`; none is on by default. |

The Rust blocks of this guide are doctests of the `examples` crate (`cargo test -p cicada-examples --doc`).

## Run it as a binary

`cicada-app` turns a set of actor types into a binary. Name the store's feature on it (`features = ["sqlite"]`,
`["postgres"]` or both; `"tui"` adds the terminal UI); none is on by default. `SyncActor` and `ReportsActor` are from
[examples/src/sync.rs](../../examples/src/sync.rs):

<!-- doctest
# use cicada_examples::sync::{Page, ReportsActor, Sync, SyncActor, SyncApi, SyncMsg};
# struct HttpSyncApi;
# impl SyncApi for HttpSyncApi {
#     async fn fetch(&self, _: &str, _: Option<&str>, _: &str) -> Page { Page { size: 0, next: None } }
# }
@@43
-->
```rust,no_run
use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use cicada::{ActorSystem, SendResult};
use cicada_app::App;

fn main() -> ExitCode {
    let token: Arc<str> = std::env::var("API_TOKEN").unwrap_or_default().into();
    App::new()
        .host(SyncActor { api: HttpSyncApi }) // the types of the binary; `--types` picks some of them
        .host(ReportsActor)
        .http(move |api, system| {
            let own_routes = Router::new()
                .route("/tenants/{tenant}/sync", post(start_sync)) // your own routes use the in-process API
                .with_state(system.clone());
            // The API has no authentication of its own: a layer covers it and what was merged before it.
            api.merge(own_routes).layer(middleware::from_fn_with_state(token, bearer))
        })
        .run() // reads the command line and the environment, returns the exit code
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
```

The binary has three kinds of commands:

```sh
myapp serve --store actors.db                              # the node and the HTTP API on 127.0.0.1:8080, until SIGTERM
myapp serve --store "$DATABASE_URL" --listen 0.0.0.0:8080   # PostgreSQL; open to every network, so add a layer first
myapp worker --store "$DATABASE_URL" --types sync           # hosts only `sync` and serves nothing
myapp get actors --server http://127.0.0.1:8080             # an operator command, over HTTP
```

- `serve` hosts the types, serves the API at the root and a public `/ready`, and stops in good order on SIGTERM or
  SIGINT: the node stops claiming and lets running turns finish within its grace period while the API still answers
  and `/ready` answers `503`; then the HTTP server stops (requests still open get 5 s), and the store is closed. It
  writes `Listening on http://<address>` when it takes requests.
- `serve` listens on the loopback address unless `--listen` says otherwise, because `App` has **no authentication of
  its own**: without a layer of the host, the API can change and delete everything. `App::http` gets the router of the
  API and the node and returns the router to serve; a layer covers what was merged before it, not what is merged after
  it. `/ready` is answered by `App` outside that router: no layer covers it, and a route `/ready` of the host is never
  reached. A route that the API already has makes axum panic: its message goes to standard error, and the command
  fails with it before the node starts.
- `worker` hosts the types and serves nothing; it has no `/ready`. `--types` picks the types of a node: `all` (the
  default), or type names with commas; `serve --types none` serves the API and hosts nothing.
- The operator commands are those of the [command line](#command-line-and-terminal-ui), over HTTP, also in a binary
  that serves. `serve` and `worker` refuse their flags `--context`, `--server`, `--token` and `-o`. A binary without
  types has neither `serve` nor `worker`, so the plain `cicada` binary never serves.
- `App::on_start` runs after the node started, in `serve` before it listens and in `worker`: to create an actor that
  must exist. Its error stops the node and fails the command. `App::observe` adds an observer (for telemetry,
  `cicada-otel`'s `ActorTelemetry`), `App::node_config` the host's `NodeConfig`, and `App::version` what `--version`
  prints. A host whose `main` is async calls `run_async`; `run_with` takes the arguments, the environment, the output
  and the stop from its caller, for tests. What the host builds before `run` is built for every command, also for
  `--help`: a host with something costly, or something an operator command lacks, asks `cicada_app::node_command`
  first.
- Exit codes: 0 for success, the help, the version and a node that was stopped; 2 for a command line that cannot be
  used (an unknown flag, a value that cannot be read, no store, a type the binary does not host); 1 for everything
  else that fails (a store that stays away, an address that is taken, a node that cannot start, an `on_start` that
  fails, an operator command that fails). `serve` and `worker` log with `tracing` to standard output, filtered by
  `RUST_LOG` (`info,sqlx=warn` when unset).

Every setting has a flag and a variable; a flag wins over its variable, and a blank variable counts as unset:

| Flag | Variable | Default | Meaning |
| --- | --- | --- | --- |
| `--store` | `CICADA_STORE` | required | `postgres://…` or `postgresql://…` (PostgreSQL), `sqlite:FILE`, or the path of a SQLite file. Another value with `://`, or one that starts with `postgres:`, is refused and never printed, as it may hold a password |
| `--store-schema` | `CICADA_STORE_SCHEMA` | `public` | the schema, on PostgreSQL |
| `--store-pool-size` | `CICADA_STORE_POOL_SIZE` | the larger of 20 and the concurrency plus 4 | the connections of this node, on PostgreSQL |
| `--retention` | `CICADA_RETENTION` | 7 days | how long outcomes are kept, also the window in which a message id is known (S10); set one value on every node |
| `--listen` | `CICADA_LISTEN` | `127.0.0.1:8080` | the address `serve` listens on |
| `--types` | `CICADA_TYPES` | `all` | the types of the node: `all`, type names with commas, or (`serve` only) `none`; `all` and `none` are keywords only as the whole value |

The settings of the node: a flag wins over its variable, the variable over the host's `NodeConfig`, and that over the
default of the library ([Configuration](core.md#configuration)):

| Flag | Variable | `NodeConfig` field |
| --- | --- | --- |
| `--node-id` | `CICADA_NODE_ID` | `node_id` |
| `--concurrency` | `CICADA_CONCURRENCY` | `concurrency` |
| `--lease-ttl` | `CICADA_LEASE_TTL` | `lease_ttl` |
| `--heartbeat-interval` | `CICADA_HEARTBEAT_INTERVAL` | `heartbeat_interval` |
| `--poll-interval` | `CICADA_POLL_INTERVAL` | `poll_interval` |
| `--shutdown-grace-period` | `CICADA_SHUTDOWN_GRACE_PERIOD` | `shutdown_grace_period` |
| `--max-turn` | `CICADA_MAX_TURN` | `default_max_turn` |
| `--max-lost-claims` | `CICADA_MAX_LOST_CLAIMS` | `max_lost_claims`: a number, or `off` |

A duration is a whole number with `ms`, `s`, `m`, `h` or `d` (`250ms`, `5s`, `7d`), or ISO 8601 (`PT5S`, `P7D`). On
PostgreSQL, `serve` and `worker` try again while the store answers that it is unavailable, as containers start in any
order: a second after each try, and no new try after a minute. One try can take seconds (about 10 s for an address
that does not answer), so the wait is about a minute and can be longer, and a wrong password counts as unavailable and
is tried again like an unreachable database. A store whose feature the binary was built without fails the command and
names the feature.

## Command line and terminal UI

`cicada-app` builds the binary `cicada`, and every binary built from `App` has the same commands. They talk to a node
only over HTTP, through `cicada-client`, so they work against any node that serves `actor_api()`; the commands that
only read also work against `actor_api_read()`. Install `cicada` from a checkout with
`cargo install --path crates/cicada-app --features tui`, or from a release with
`cargo install --git https://github.com/helico-tech/cicada.rs --tag vX.Y.Z cicada-app --features tui`; without
`--features tui` it has no terminal UI and builds neither `ratatui` nor `crossterm`. Run it from a checkout with
`cargo run -p cicada-app --features tui -- <command>`. The help, the usage errors and the hints name the binary as it was
called (`myapp config set-context`), and a usage error exits with code 2.

The help of every command, as `--help` prints it, is in [the reference of the command line](../reference/cli.md).

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
holds tokens. A config path that is a symlink (a dotfiles setup) is written through: the file it points to gets the
new text, a relative link is read from the folder of the link, a target that is not there yet is created, and the link
stays a link.

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

**Terminal UI.** `cicada tui`, in a binary built with the feature `tui` (with the same flags, e.g.
`cicada --server http://localhost:8080 --token secret tui` against the sample service with `API_TOKEN=secret`) opens on
the actors. The header shows the context, the server, the
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

## Sample service

`sample-service` is a runnable node on `cicada-app` (binary `cicada-sample-service`, Docker image from
`sample-service/Dockerfile`) hosting the examples and the end-to-end actors; see
[sample-service/](../../sample-service/). `cicada-sample-service serve --store actors.db` runs it; it takes the flags
and variables of
[Run it as a binary](#run-it-as-a-binary), picks its types with `--types` (`CICADA_TYPES`, type names such as
`e2e.counter`), and creates the singletons of the types it hosts at the start. It hosts the three ready-made actors; a
node that hosts the dead letters creates `cicada.dead-letters/_` and its watch on every type at the start. `GET /` names
the node and its types, without authentication. It reads two variables of its own:

| Variable | Meaning | Default |
| --- | --- | --- |
| `API_TOKEN` | when set, the HTTP API requires `Authorization: Bearer <token>` (`/` and `/ready` stay public) | — |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | when set, a node exports traces and metrics there over OTLP (`ActorTelemetry`); only HTTP/protobuf, so `OTEL_EXPORTER_OTLP_PROTOCOL` must be unset or `http/protobuf` | — |

Its actors record the node id, so it reads `CICADA_NODE_ID` itself; a `--node-id` flag that differs from it fails the
start. Its variables before 0.13.0 are replaced by those of `cicada-app`, and for this release a node refuses to start
while one of them is set, naming the new one: `PORT` is `CICADA_LISTEN`, `DATABASE_URL` and `SQLITE_PATH` are
`CICADA_STORE`, `DATABASE_SCHEMA` is `CICADA_STORE_SCHEMA`, `DATABASE_POOL_SIZE` is `CICADA_STORE_POOL_SIZE`,
`HOSTED_TYPES` (with its groups) is `CICADA_TYPES` (with type names), and `NODE_ID`, `CONCURRENCY`, `LEASE_TTL`,
`HEARTBEAT_INTERVAL`, `POLL_INTERVAL`, `SHUTDOWN_GRACE_PERIOD` and `MAX_TURN` get the prefix `CICADA_`. The image
runs `serve` with `CICADA_LISTEN=0.0.0.0:8080`, `CICADA_STORE=/data/actors.db` and `CICADA_SHUTDOWN_GRACE_PERIOD=3s`.

The root `docker-compose.yml` starts a PostgreSQL for development and tests
(`postgres://cicada:cicada@localhost:55432/cicada`).

## Running it

How to size, deploy, upgrade and watch the nodes of such a binary, and what to do on call, is in
[the operations guide](../operations.md): [Sizing](../operations.md#sizing), [Deploying](../operations.md#deploying),
[Upgrading](../operations.md#upgrading) and [On call](../operations.md#on-call).
