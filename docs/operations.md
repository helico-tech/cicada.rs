# Operating Cicada for Rust

How to size, deploy, upgrade and watch Cicada nodes. The design is in [architecture.md](architecture.md), the API and
the settings in the [README](../README.md). Settings are `NodeConfig`, `PostgresStoreConfig` and `SqliteOptions`, handlers are tokio
futures, logs go through `tracing`, and the host owns the store and the node's life cycle.

## Sizing

- **`concurrency`** (default 16) is how many turns one node runs at once. Each running turn is a tokio task (plus a
  heartbeat task), so idle waiting costs nothing, but a handler that blocks holds one of the runtime's worker threads
  (by default one per CPU core) until it returns. Run blocking calls (blocking I/O, `std::thread::sleep`, long
  computations) in `tokio::task::spawn_blocking`. Several blocking handlers can take every worker thread, and then
  heartbeats and the dispatch loop stall too: leases expire and turns are run twice. A handler that is still running
  a while after its maximum turn or after its lease was lost is logged as a warning (`… it does not yield (blocking
  code? use spawn_blocking)`).
- **PostgreSQL pool** (`PostgresStoreConfig::max_pool_size`, default 20): a running turn uses a connection for its
  claim, heartbeat and commit, and HTTP requests use the same pool. Keep it at least `concurrency + 4`. A pool that is
  too small delays lease renewals, and a renewal that waits too long makes the node give up the turn. Mind the server
  side too: `nodes × max_pool_size` plus up to two connections per node for the wake-up listener (`LISTEN` and its
  probe) must stay under PostgreSQL's `max_connections`.
- **`lease_ttl`** (30 s) is how long a dead node's turn waits before another node re-runs it. Shorter means faster
  recovery but more heartbeat writes (every `lease_ttl / 3`) and more risk that a stalled runtime or a long pause
  loses the lease.
- **Maximum turn** (15 min, `NodeConfig::default_max_turn`, per type `Actor::MAX_TURN` or `ActorType::max_turn`)
  bounds a turn. A turn that runs into it is `failed("timeout")`, or retried when the message has a retry policy. A
  handler that does not yield is cancelled only when it returns.
- **SQLite** has one writer: fine for one node, or a few processes on one host. Use PostgreSQL for more.

## Deploying

- **Graceful stop.** Nothing handles signals for you: the host waits for `SIGTERM` (and Ctrl-C), calls
  `system.stop()`, and closes the store afterwards. On stop a node stops claiming, lets running turns finish within
  `shutdown_grace_period` (30 s), then cancels them and releases their leases; a cancelled turn that does not end
  within `shutdown_cancel_timeout` (5 s) is left to its lease. Give the platform's stop timeout more than the sum
  (e.g. Kubernetes `terminationGracePeriodSeconds`, compose `stop_grace_period`); otherwise the process is killed and
  its turns wait out their lease. A process killed without `stop` loses nothing either: its turns re-run elsewhere
  after the lease TTL.

  ```rust,no_run
  use std::sync::Arc;
  use std::time::Duration;

  use cicada::ActorSystem;
  use cicada::runtime::NodeConfig;
  use cicada::store::Store;
  use cicada_store_postgres::{PostgresStore, PostgresStoreConfig};
  use tokio::signal::unix::{SignalKind, signal};

  #[tokio::main]
  async fn main() -> anyhow::Result<()> {
      tracing_subscriber::fmt().with_env_filter("info").init(); // or EnvFilter::from_default_env() for RUST_LOG
      let config = PostgresStoreConfig { max_pool_size: 36, ..PostgresStoreConfig::for_schema("orders") };
      let store = Arc::new(PostgresStore::connect(&std::env::var("DATABASE_URL")?, config).await?);
      let node = NodeConfig { concurrency: 32, shutdown_grace_period: Duration::from_secs(30), ..NodeConfig::default() };
      let system = ActorSystem::builder(store.clone())
          // .host(…) the types this node runs
          .config(node)
          .build();
      system.start().await?;
      // … serve the HTTP API (see the README) …

      let mut terminate = signal(SignalKind::terminate())?;
      tokio::select! {
          _ = tokio::signal::ctrl_c() => {}
          _ = terminate.recv() => {}
      }
      system.stop().await; // turns finish or are cancelled; leases released
      store.close().await; // after the node, never before
      Ok(())
  }
  ```

  When the node serves HTTP, stop it inside axum's `with_graceful_shutdown` future, as the README's quick start does:
  the API keeps answering while turns finish, and `/ready` answers `503` from the moment `stop` begins.
- **Rolling deploys** work as long as the contracts stay equal: a node refuses to start when a type's contract
  differs from the registered one. Check that in CI with contract snapshots (`cicada_test::assert_contracts_match`,
  README, Testing) and keep stored states readable with state samples (`assert_state_decodes`).
- **Schema migrations** run when a store is opened (`SqliteStore::open`, `PostgresStore::connect`), so the first
  node of a new version migrates the database. On PostgreSQL concurrent starts serialise on an advisory lock, and a current schema runs no DDL, so a node may connect
  as a user without `CREATE` once the schema was migrated. A store does not open a schema newer than its own (`StoreError::Invalid`). Start the new nodes
  first, then stop the old ones, and do not restart old nodes after the migration (see [Upgrading](#upgrading)).
- **Send-only nodes** (a node that hosts nothing) serve the whole API for every registered type and are a safe place
  for public traffic: they never run turns.
- **Probes.** The API's `GET /health` usually sits behind the host's authentication layer, where a probe has no
  credentials. Merge `actor_health(system, "/ready")` outside it: it answers like `/health` (`200` with the node id and
  state when the node runs and reaches its store, else `503`). Use it for **readiness** only. A store outage makes
  every node unready, and restarting them does not bring the store back, so do not use it for liveness. A Kubernetes
  example for a node with the default 30 s grace period:

  ```yaml
  spec:
    terminationGracePeriodSeconds: 45     # > shutdown_grace_period (30 s) + shutdown_cancel_timeout (5 s)
    containers:
      - name: node
        readinessProbe:
          httpGet: { path: /ready, port: 8080 }
          periodSeconds: 5
          failureThreshold: 3
        livenessProbe:                     # the process answers HTTP; no store check
          tcpSocket: { port: 8080 }
          periodSeconds: 10
          failureThreshold: 6
  ```

  A rolling update of two pods on one SQLite file (one host, a `ReadWriteOnce` volume) hands over cleanly: the old pod
  finishes its running turn within the grace period and releases its lease, and the new pod continues.

### Changing a contract

A node refuses to start when one of its types has a contract other than the registered one, so mixed versions never
decode each other's messages differently. That holds also for compatible changes, such as a new field with a default,
and for a dependency upgrade that changes the generated schemas (schemars). Change a type in one of two ways:

- **Add, then switch.** Keep the old type and add the new one under a **versioned type name**
  (`billing.v2.invoices`). New instances are created at the new address; senders switch to it in a later
  release. Old instances keep running on their old contract until they are done, or a handler of the old type moves
  them: it creates the new instance with its state converted and stops itself (`turn.create(&new_ref, &converted)?`,
  then `turn.stop()`), in one atomic turn.
- **Stop the world** (a single node, or a deploy that stops every node before starting new ones): reset the type's
  registration in the store and start the new version. Use contract snapshots to see the change in CI and state
  samples to prove old states still decode. Stop every node that hosts the type first: a running node registers its
  type again at its next start, and one that is still up keeps running on the old contract. Then delete the actors
  of the type (`DELETE /actors/{type}/{key}`) and remove its registration with `DELETE /types/{name}`; the call is
  refused with `409` while an actor of the type exists. Start the new version, which registers its contract.

## Upgrading

Stores write schema version 1 and apply any missing migrations in place when they open a database. Migrations are
additive, so nodes of the previous version keep working during a rolling deploy.

**Order of a rolling deploy.** The first node of the new version migrates the database; from then on a node of the
old version refuses to start (it does not open a schema newer than its own), while old nodes that are already running
keep working. So: start the new nodes, then stop the old ones. Do not restart an old node after the migration (a
crash-looping or rescheduled old pod will not come back), and do not roll back against a migrated database.

**Upgrade nodes before clients.** Nodes decode request bodies strictly, so a request field a node does not know yet
is a `400`. `ActorClient` leaves absent optional fields out of request bodies (no `"retry": null`), so a new client
works against an older node as long as it does not use a newer feature.

## On call

- **Health.** `GET /health` is 200 while the node runs and its store answers, 503 otherwise (also while it stops).
  Use it for readiness; for liveness, prefer a check that does not fail when only the database is down.
- **Failures.** `GET /outcomes?status=failed&since=…` lists failed messages with envelope and error, kept for the outcome
  retention (7 days). Without `status` it lists every finished message. For longer, host `DeadLettersActor` and run `FailureWatcher` (`cicada-extras`): failures become
  dead letters that an operator redrives or discards with a message.
- **A message that keeps killing its node** (out of memory, an abort, a native crash) is claimed again forever: its
  `attempt` climbs while `failures` stays put (`GET /messages/{id}`). Install `CrashLoopGuard` (`cicada-extras`) to
  fail such a message after N claims without a commit; set N above what deploys during long turns cause, since every
  lost or released lease counts.
- **Stuck instances.** An instance whose state no longer decodes fails every turn: its messages end `failed` one by
  one. Fix the state type (add `#[serde(default)]`), or delete and re-create the instance (`DELETE`, then
  `PUT /actors/{type}/{key}`; the delete purges its mailbox).
- **Store down.** Nothing proceeds and HTTP answers 503 `store-unavailable`; `ActorClient` retries sends with the same
  id. Nodes recover on their own when the store returns: the dispatch loop retries claims after `poll_interval`, and
  the PostgreSQL wake-up listener reconnects with a back-off (from 250 ms, doubling up to 30 s),
  polling meanwhile.

### Node table

Every node, send-only ones too, writes a row to the `nodes` table every 10 s, with its id and hosted types; `GET /nodes`
and `GET /nodes/{node}` read it. Status is derived from the age of the last heartbeat in the store's clock: `alive`
(under 30 s), `suspect` (under 2 min), `dead`. Any node's heartbeat removes rows older than a day, and a clean stop
removes the node's own row. The table is for display only: leases decide who runs what, so a wrong status cannot cause
overlapping turns. A stopped turn is not forced away from a node that is alive: `POST
/actors/{type}/{key}/turn/cancel` asks the owning node to cancel it within one heartbeat, and a dead node's lease
expires by itself within the lease TTL.

### Logs

The runtime and the stores log with [`tracing`](https://docs.rs/tracing); the host installs a subscriber
(`tracing_subscriber::fmt()`, filtered with `RUST_LOG` through `EnvFilter`). Nothing is printed without one. Events
worth an alert:

| Level | Message | Meaning |
| --- | --- | --- |
| warn | `Claiming failed; retrying after …` | The store is unreachable or failing. |
| warn | `Commit of … failed; retrying …`, `Commit of … refused: …` | A commit could not be written; the turn re-runs after its lease, or the message ends `failed`. |
| info | `Commit of … rejected: lease lost` | A turn outlived its lease (a stalled runtime, a pause, a slow store); another node may have run it. |
| warn | `Handler of … is still running … it does not yield` | Blocking code in a handler; move it to `spawn_blocking`. |
| error | `Turn of … panicked …`, `A turn task failed` | A panic outside the handler (a hook, the runtime): a bug. A handler's own panic is an ordinary handler failure (`panic: …`). |
| warn | `on_handler_error of plugin … failed; ignored`, `on_outcome of plugin … failed; ignored` | A plugin hook failed. |
| warn | `Retention sweep failed`, `Store wake-ups ended; falling back to polling` | The node keeps working, slower or with more stored data. |
| warn | `Ignoring the retry policy of …` | A stored policy that cannot be decoded is read as no policy. |
| info | `Cancelling N turns after the grace period of …` | A stop that cut running turns off. |

## Store tuning

PostgreSQL (`PostgresStoreConfig`):

| Setting | Default | Notes |
| --- | --- | --- |
| `schema` | `public` | Created on start when absent. Separate schemas let environments or tests share a database. |
| `retention` | 7 days | Outcome retention and the dedup window of message ids. |
| `max_pool_size` | 20 | At least `concurrency + 4`; see [Sizing](#sizing). |
| `socket_timeout` | 30 s | Longest single transaction attempt; then it fails and its connection is closed, so a dead peer without a TCP reset cannot block a claim, commit or stop. Zero disables it. |

Connection parameters (`sslmode`, `sslrootcert`, `application_name`, `options`, …) go in the URL;
`PostgresStore::connect_with(PgConnectOptions, config)` takes sqlx options for anything a URL cannot carry. Store time
is the server's `clock_timestamp()`, read after each transaction's row locks, so run PostgreSQL with a disciplined
clock (NTP).

SQLite (`SqliteOptions`):

| Setting | Default | Notes |
| --- | --- | --- |
| `retention` | 7 days | As for PostgreSQL. |
| `busy_timeout` | 5 s | How long a write waits for another process's lock before it fails as unavailable. |
| `clock` | system clock | The store clock; a virtual one in tests. |

The file runs in WAL mode with `synchronous = FULL`. Keep it on a local disk, not a network file system. Store time
never steps back (a persisted high-water mark), but a wall clock that jumps forward moves it forward too: leases
expire early and delayed messages fall due early, so run the host with NTP slewing.

## Telemetry

Install `ActorTelemetry` (`cicada-otel`, README, OpenTelemetry) to get a span per turn and the metrics
`cicada.turn.duration` and `cicada.message.lag`. Useful signals:

- **Lag** (`cicada.message.lag`) rising for one type: too few nodes host it, `concurrency` is too low, or its turns
  got slower. A message is claimed at most `poll_interval` late when nodes are idle.
- **Failed turns** (`cicada.turn.duration` count with `cicada.outcome="failed"`), by type.
- **Retries** (`cicada.outcome="rescheduled"`) climbing: a dependency is failing and senders' retry policies are
  absorbing it.
- **Abandoned turns** (spans with `cicada.outcome="abandoned"`): turns that lost their lease or were cut off by a stop.
- Mailbox depth is not a metric (it needs a store query); `GET /stats?prefix=` counts the actors and pending messages
  under a prefix and names the delivery time of the oldest due message, and `GET /messages?prefix=` lists them. A
  call reads every row under the prefix, so poll it rarely, and with a prefix on a big store.
