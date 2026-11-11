# Operating Cicada for Rust

How to size, deploy, upgrade and watch Cicada nodes. The design is in [architecture.md](architecture.md), the API and
the settings in the [README](../README.md). Settings are `NodeConfig`, `PostgresOptions` and `SqliteOptions`, handlers are tokio
futures, logs go through `tracing`, and the host owns the store and the node's life cycle.

## Sizing

- **`concurrency`** (default 16) is how many turns one node runs at once. Each running turn is a tokio task (plus a
  heartbeat task), so idle waiting costs nothing, but a handler that blocks holds one of the runtime's worker threads
  (by default one per CPU core) until it returns. Run blocking calls (blocking I/O, `std::thread::sleep`, long
  computations) in `tokio::task::spawn_blocking`. Several blocking handlers can take every worker thread, and then
  heartbeats and the dispatch loop stall too: leases expire and turns are run twice. A handler that is still running
  a while after its maximum turn or after its lease was lost is logged as a warning (`… it does not yield (blocking
  code? use spawn_blocking)`).
- **PostgreSQL pool** (`PostgresOptions::max_pool_size`, default 20): a running turn uses a connection for its
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
  use std::time::Duration;

  use cicada::runtime::NodeConfig;
  use cicada::store::PostgresOptions;
  use cicada::{ActorSystem, Store};
  use tokio::signal::unix::{SignalKind, signal};

  #[tokio::main]
  async fn main() -> anyhow::Result<()> {
      tracing_subscriber::fmt().with_env_filter("info").init(); // or EnvFilter::from_default_env() for RUST_LOG
      let options = PostgresOptions { max_pool_size: 36, ..PostgresOptions::for_schema("orders") };
      let store = Store::postgres(&std::env::var("DATABASE_URL")?, options).await?;
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
- **Schema migrations** run when a store is opened (`Store::sqlite`, `Store::postgres`), so the first
  node of a new version migrates the database. On PostgreSQL concurrent starts serialise on an advisory lock, and a current schema runs no DDL, so a node may connect
  as a user without `CREATE` once the schema was migrated. A store does not open a schema newer than its own (`StoreError::Invalid`). Start the new nodes
  first, then stop the old ones, and do not restart old nodes after the migration; a release whose migration is not
  additive needs every old node stopped first (see [Upgrading](#upgrading)).
- **A migration that violates a constraint** of the database (two old keys that become one name in migration 3) fails
  the open on both stores with `StoreError::Invalid`, which names the schema version it could not reach and holds the
  database's message. The database stays at its version, and a node stops instead of retrying, as trying again cannot
  help: fix the data and start it again. A lost connection, a locked SQLite file, a PostgreSQL deadlock and a missing
  privilege stay `StoreError::Unavailable`, which a node may retry.
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
and for a dependency upgrade that changes the generated schemas (schemars). A reworded doc comment is not a change: the
hash leaves out the schemas' `description`s (ADR 0017). `GET /types` keeps showing the descriptions of the first
registration until the type is registered again. Change a type in one of two ways:

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

Stores write schema version 4 and apply any missing migrations in place when they open a database. Migrations are
additive, so nodes of the previous version keep working during a rolling deploy, unless a release says otherwise: a
migration that is not additive breaks the nodes of the previous version, so every old node is stopped before the first
new one starts. The changelog says so for each such release.

**Version 0.4.0 is such a release.** Its migration 2 (every actor has state, ADR 0018) gives every actor without a
state the state `{}`, makes the state column required, and removes the registrations of the types that were
stateless; an old node that writes an actor without a state fails. Stop every node of the old version, then start the
new ones, the nodes that host the formerly stateless types first. Until a node that hosts such a type starts, its
actors can be read and deleted, and messages already in a mailbox wait; a create of one and a send to one from outside
a turn are refused (over HTTP `404 no-such-type`), a retry of a failed message of one is accepted and waits, and a turn
on another node that creates one ends `failed` without a retry. The
changelog of 0.4.0 lists the steps, including the code changes.

**Version 0.6.0 is such a release.** A key is one name (ADR 0021), so key fields leave every contract and every
contract hash changes. Its migration 3 removes every registration and rewrites `%3A` to `:` in the address columns:
a `:` is an ordinary character of a name now, so `account/acme:7` is the address of the name `acme:7`. Before the
upgrade, find the old keys of two fields that become one name with the query in the changelog of 0.6.0 and delete one
actor of each pair with the old version; otherwise the migration fails and the database stays at version 2. A store
of 0.7.0 or later reports that on both stores as an invalid store that names schema version 3 and holds the database's
message, and a node stops; a store of 0.6.0 reported it as unavailable (on SQLite the store did not open), so a node
that retried an unavailable store kept retrying. Stop every
node of the old version, then start the new ones: the nodes that host a type before the nodes whose turns create
actors of it, and nodes that only send last. Until a node that hosts a type starts, that type is not registered, with
the effects described for 0.4.0 above; where two kinds of nodes create actors of each other's types, hold back the
messages that start such turns until every node is up. Upgrade `ActorClient` users together with the nodes: an old
client cannot decode the views of a new node. A prefix that holds the encoded colon itself (`x%3A` in code,
`%253A` in a URL) is written with `:`. The changelog of
0.6.0 lists the steps, including the code changes and what happens to a ref that changes its shape.

**Version 0.8.0 is such a release.** An actor may have a parent (ADR 0019), and a delete or a stop removes the actor's
descendants with it. Its migration 4 adds the column `actors.parent` and an index on it; every existing actor is
independent. A node of 0.7.0 that opens the migrated database refuses it, but one that is still running does not
notice: its deletes and stops remove one row and leave the children behind, as orphans that never go away. So stop
every node of the old version, then start the new ones. Upgrade `ActorClient` users together with the nodes: the
view of an actor has the field `parent`.

**Switching a type to `create_child`.** An actor created with `turn.create` stays independent: a later `create_child`
of it by its creator does not adopt it, but fails the creator's message (the commit is rejected, and a handler cannot
test for that). So switch a parent type from `create` to `create_child` only when no parent of it is left that created
actors the old way: start its data anew, or let those parents finish first. New parents can use `create_child` from
the start.

**Order of a rolling deploy.** For an additive migration: the first node of the new version migrates the database;
from then on a node of the old version refuses to start (it does not open a schema newer than its own), while old nodes
that are already running keep working. So: start the new nodes, then stop the old ones. Do not restart an old node
after the migration (a crash-looping or rescheduled old pod will not come back), and do not roll back against a
migrated database.

**Upgrade nodes before clients.** Nodes decode request bodies strictly, so a request field a node does not know yet
is a `400`. `ActorClient` leaves absent optional fields out of request bodies (no `"retry": null`), so a new client
works against an older node as long as it does not use a newer feature. A release whose views lose a field that old
clients require, as 0.6.0 does, needs its clients upgraded together with the nodes; the changelog says so.

## Releasing

A release is a tag `vX.Y.Z` on `main` (ADR 0016). All crates share the version in `[workspace.package]`.

1. On `main`, set the version in `Cargo.toml` and run `cargo check`, which updates `Cargo.lock`. A breaking change raises
   the minor number while the version is 0.x, anything else the patch number; the README says what counts as breaking.
   `docs/openapi.json` carries the version: regenerate it (a test says how when it is stale) and update the README's
   `tag = "vX.Y.Z"` lines.
2. In `CHANGELOG.md`, replace `unreleased` in the heading of that version with today's date, as `YYYY-MM-DD`.
3. Commit and push to `main`, and wait for CI.
4. Run `scripts/release-check.sh vX.Y.Z`. It prints the release notes, or says what is wrong.
5. `git tag -a vX.Y.Z -m vX.Y.Z` and `git push origin vX.Y.Z`.

The push starts `.github/workflows/release.yml`: the same check, CI, then the end-to-end suite, and only then a GitHub
Release with the changelog section as its notes. If a check fails, no release exists yet: delete the tag
(`git push origin :refs/tags/vX.Y.Z` and `git tag -d vX.Y.Z`), fix the cause on `main` and tag again. A tag with a
GitHub Release is never moved or deleted; a correction is a new version.

When the next change lands, add a section `## X.Y.Z — unreleased` to the changelog and set that version in `Cargo.toml`.

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
  `PUT /actors/{type}/{key}`; the delete purges its mailbox and removes its descendants).
- **What a delete removes.** `DELETE /actors/{type}/{key}` (`cicada delete actor`) removes the actor and every
  descendant: the actors its turns created with `create_child`, theirs, and so on, in one transaction. Every pending
  message of them ends `purged`, a turn running on one commits nothing, and the delete counts the purged messages of
  the whole tree. Actors created with `create` are independent and stay. See what an actor owns first with
  `GET /actors/{type}/{key}/children` (`cicada get children`, or the `children` tab of an actor in `cicada tui`), one
  level at a time.
- **A large tree** is removed in one transaction that writes one `purged` outcome per pending message of every
  descendant; on SQLite it holds the single writer meanwhile. On PostgreSQL a removal that takes longer than
  `socket_timeout` (30 s) cannot finish: every delete of the tree answers `503 store-unavailable`, and a `stop()` of
  its root fails and is handled again and again. There are two ways out: remove the tree from the leaves up, listing
  each level with `GET /actors/{type}/{key}/children` and deleting the deepest actors first, or raise `socket_timeout`
  (`PostgresOptions`) for the node that runs the removal. A tree of many thousands of actors with full mailboxes is
  better removed bottom-up by its own code.
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

PostgreSQL (`PostgresOptions`, feature `postgres`):

| Setting | Default | Notes |
| --- | --- | --- |
| `schema` | `public` | Created on start when absent. Separate schemas let environments or tests share a database. |
| `retention` | 7 days | Outcome retention and the dedup window of message ids. |
| `max_pool_size` | 20 | At least `concurrency + 4`; see [Sizing](#sizing). |
| `socket_timeout` | 30 s | Longest single transaction attempt; then it fails and its connection is closed, so a dead peer without a TCP reset cannot block a claim, commit or stop. It also bounds the largest tree a delete or a stop can remove ([On call](#on-call)). Zero disables it. |

Connection parameters (`sslmode`, `sslrootcert`, `application_name`, `options`, …) go in the URL of
`Store::postgres(url, options)`; it is the one way to open the store, and a setting a URL cannot carry is added to
`PostgresOptions` when a host needs it. For TLS see [TLS for PostgreSQL](#tls-for-postgresql). Store time
is the server's `clock_timestamp()`, read after each transaction's row locks, so run PostgreSQL with a disciplined
clock (NTP).

SQLite (`SqliteOptions`, feature `sqlite`):

| Setting | Default | Notes |
| --- | --- | --- |
| `retention` | 7 days | As for PostgreSQL. |
| `busy_timeout` | 5 s | How long a write waits for another process's lock before it fails as unavailable. |
| `clock` | system clock | The store clock; a virtual one in tests. |

The file runs in WAL mode with `synchronous = FULL`. Keep it on a local disk, not a network file system. Store time
never steps back (a persisted high-water mark), but a wall clock that jumps forward moves it forward too: leases
expire early and delayed messages fall due early, so run the host with NTP slewing.

### TLS for PostgreSQL

The feature `postgres` builds the driver with TLS (rustls); there is nothing else to switch on. Ask for TLS with
`sslmode` in the URL:

- `disable`, `allow`: plain text; the driver never tries TLS.
- `prefer` (the default): TLS without checking the server's certificate when the server offers TLS, plain text when
  it does not. Once the server has offered TLS, a failed handshake fails the connection; it does not go back to plain
  text (`sslmode=disable` does).
- `require`: TLS or no connection; the server's certificate is not checked.
- `verify-ca`: as `require`, and the certificate must be signed by a trusted CA.
- `verify-full`: as `verify-ca`, and the certificate must name the host of the URL.

The trusted CAs are the built-in list of public CAs (webpki-roots) and those in the PEM file that `sslrootcert` names
(`…?sslmode=verify-full&sslrootcert=/etc/ssl/db-ca.pem`); the certificate store of the operating system is not read.
A database rarely has a certificate of a public CA: one with a private or cloud-provider CA needs `sslrootcert`.
`sslcert` and `sslkey` name the files of a client certificate and its key, for a server that asks for one. For each of
the four that the URL does not name, the driver reads `PGSSLMODE`, `PGSSLROOTCERT`, `PGSSLCERT` or `PGSSLKEY` from the
environment, so the default mode can come from there. The default `prefer` neither insists on TLS nor checks the
server, so set the mode on purpose: `sslmode=verify-full`, or at least `require`.

The crates of Cicada build rustls on the crypto provider `aws-lc-rs`; a host that adds a library that builds rustls on
`ring` has two providers, and must install a default at start
(`rustls::crypto::aws_lc_rs::default_provider().install_default()`), or every call that leaves the choice to rustls
panics.

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
