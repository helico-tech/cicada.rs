# Operating Cicada for Rust

How to size, deploy, upgrade and watch Cicada nodes. The design is in [architecture.md](architecture.md), the API and
the settings in the guides of [the core](guide/core.md), [HTTP](guide/http.md) and [the app](guide/app.md).
Settings are `NodeConfig`, `PostgresOptions` and `SqliteOptions` ([Configuration](guide/core.md#configuration)), or
the flags and `CICADA_…` variables of a binary built from `cicada-app` ([Run it as a
binary](guide/app.md#run-it-as-a-binary)); handlers are tokio futures, logs go through `tracing`, and the host owns
the store and the node's life cycle, or hands them to `App`.

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
  probe) must stay under PostgreSQL's `max_connections`. A binary built from `cicada-app` sizes the pool as the larger
  of 20 and `concurrency + 4` unless `--store-pool-size` (`CICADA_STORE_POOL_SIZE`) says otherwise; set it per
  deployment so that the nodes together stay under the server's limit.
- **`lease_ttl`** (30 s) is how long a dead node's turn waits before another node re-runs it. Shorter means faster
  recovery but more heartbeat writes (every `lease_ttl / 3`) and more risk that a stalled runtime or a long pause
  loses the lease.
- **Maximum turn** (15 min, `NodeConfig::default_max_turn`, per type `Actor::MAX_TURN` or `ActorType::max_turn`)
  bounds a turn. A turn that runs into it is `failed("timeout")`, or retried when the message has a retry policy. A
  handler that does not yield is cancelled only when it returns.
- **SQLite** has one writer: fine for one node, or a few processes on one host. Use PostgreSQL for more.

## Deploying

- **`serve` and `worker`.** A binary built from `cicada-app` runs a node with `serve` (the node and the HTTP API) or
  `worker` (the node only), configured by flags and `CICADA_…` variables:

  ```sh
  myapp serve --store "$DATABASE_URL" --listen 0.0.0.0:8080 --concurrency 32   # a node that serves; add a layer first
  myapp serve --store "$DATABASE_URL" --listen 0.0.0.0:8080 --types none       # a node that only serves HTTP
  myapp worker --store "$DATABASE_URL" --types billing.invoices                  # a node that only runs turns
  ```

  `serve` listens on `127.0.0.1:8080` unless `--listen` (`CICADA_LISTEN`) says otherwise, so in a container nothing
  answers from outside until it is set. The API has **no authentication** unless the host adds a layer with
  `App::http`: it can change and delete everything, so open it to a network only behind one. `serve` and `worker`
  refuse `--token`, which belongs to the operator commands, so it cannot look like protection. `/ready` stays public.
  Set the same `--retention` (`CICADA_RETENTION`, 7 days by default) on every node of a system: it is how long
  outcomes are kept and the window in which a message id is known. On PostgreSQL a node waits about a minute for a
  database that is not there yet, so containers may start in any order: it tries again a second after each try and
  starts no try after a minute, but one try can take seconds (about 10 s for an address that does not answer), so a
  node that gives up may do so after more than a minute. A wrong password counts as unavailable and is tried again for
  the whole wait, like an unreachable database; the warnings in the log name the error. Exit codes: 0 after a stop, 2
  for a command line or a variable that cannot be used, 1 for a node that cannot start.
- **Graceful stop.** `serve` and `worker` stop on `SIGTERM` and `SIGINT`. `serve` stops the node first: it claims no
  more and lets running turns finish within the grace period, while the API keeps answering and `/ready` answers
  `503`; then the HTTP server stops, requests still open get 5 s and are cut off after that, and the store is closed.
  `worker` stops the node and closes the store. A host that uses the layers directly handles signals itself: it waits
  for `SIGTERM` (and Ctrl-C), calls `system.stop()`, and closes the store afterwards. On stop a node stops claiming,
  lets running turns finish within `shutdown_grace_period` (30 s), then cancels them and releases their leases; a
  cancelled turn that does not end
  within `shutdown_cancel_timeout` (5 s) is left to its lease. Give the platform's stop timeout more than the sum
  (e.g. Kubernetes `terminationGracePeriodSeconds`, compose `stop_grace_period`); otherwise the process is killed and
  its turns wait out their lease. A process killed without `stop` loses no message: its turns re-run elsewhere after
  the lease TTL. But each of them is a lost claim of its message, and a message with five lost claims ends `failed`
  ([The claim limit](#the-claim-limit)), so a deploy that kills nodes as a matter of routine uses up that limit.

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
      // … serve the HTTP API (see the HTTP guide) …

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

  When the node serves HTTP, stop it inside axum's `with_graceful_shutdown` future, as the axum service of
  [the HTTP guide](guide/http.md#run-it-in-an-axum-service) and `serve` do: the API keeps answering while turns
  finish, and `/ready` answers `503` from the moment `stop` begins.
  With the HTTP server first, one request that stays open (a route that streams) would hold up the stop of the node
  until the platform kills the process, and every turn running then would be a lost claim.
- **Rolling deploys** work as long as the contracts stay equal: a node refuses to start when a type's contract
  differs from the registered one. Check that in CI with contract snapshots (`cicada_test::assert_contracts_match`,
  [Contract snapshots](guide/core.md#contract-snapshots)) and keep stored states readable with state samples
  (`assert_state_decodes`).
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
  credentials. Merge `actor_health(system, "/ready")` outside it (`serve` answers `/ready` outside the host's router by
  itself): it answers like `/health` (`200` with the node id and state when the node runs and reaches its store, else
  `503`). Use it for **readiness** only. A `worker` serves nothing, so it has **no readiness check**: whatever runs it
  sees only the process, and a worker that cannot reach its store keeps running and retries. A store outage makes
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

A type that starts to watch adds the variants `Failed(cicada::Failed)` and `Stopped(cicada::Stopped)` to its message
family, so its contract changes like any other: deploy it in one of the two ways above. The schema of `Failed` is part
of the contract of every type that watches; it gained the field `cause` in 0.14.0, and a new value of the cause would
change it again (ADR 0028).

## Upgrading

Stores write schema version 6 and apply any missing migrations in place when they open a database. Migrations are
additive, so nodes of the previous version keep working during a rolling deploy, unless a release says otherwise: a
migration that is not additive breaks the nodes of the previous version, so every old node is stopped before the first
new one starts. The changelog says so for each such release. From 0.11.0 on both stores name the columns they read, so
a later migration that only adds a column with a default can be additive: the statements of a running node of the
release before keep working while it is applied.

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
every node of the old version, then start the new ones. Clients of 0.7.0 keep working: they ignore the new field
`parent` of an actor's view.

**Version 0.9.0 is such a release.** A watcher is told of the failures and removals of the actors it watches, in the
transaction of the event (ADR 0023). Its migration 5 adds the table `watches` and the column `messages.notification`;
no existing message is a notification and no watch is stored. A node of 0.8.0 that opens the migrated database refuses
it, but one that is still running does not notice: a message that fails there tells no watcher, and an actor it removes
keeps its watches as a watcher. So stop every node of the old version, then start the new ones. Clients of 0.8.0 keep
working: they ignore the new field `notification` of a message's view. A host that ran `FailureWatcher` kept it until
0.14.0, which replaces it with a watch.

**Version 0.10.0 changes no schema.** A turn can pause, resume, purge and delete another actor (ADR 0024); its commit
applies those effects, and an actor on a node of 0.9.0 is paused or deleted as it would be from outside. So nodes of
0.9.0 and 0.10.0 can share a database during a rolling deploy, and clients need no upgrade.

**Version 0.11.0 is such a release.** A message counts its lost claims, and a node fails a message at its claim limit
(ADR 0025, [The claim limit](#the-claim-limit)). Its migration 6 adds the column `messages.lost_claims`; no existing
message starts with a lost claim. A node of 0.10.0 that opens the migrated database refuses it, but one that is still
running does not notice: it counts no lost claim, so the claims it replaces escape the limit, and its statements that
read a message with `*` break on the changed table. On PostgreSQL they fail ("cached plan must not change result
type") until the connection is replaced; on SQLite the worker of the driver (sqlx) crashes on them, and the store
answers unavailable. So stop every node of the old version, then start the new ones. Clients of 0.10.0 keep working:
they ignore the new field `lostClaims` of a message's view. A node that had no `CrashLoopGuard` now has a limit of 5;
the changelog of 0.11.0 lists the code changes.

**Version 0.12.0 changes no schema.** A send and a create over HTTP are checked against the registered JSON Schema of
their type on every node (ADR 0026), so nodes of 0.11.0 and 0.12.0 can share a database; during a rolling deploy a node
of 0.11.0 still answers as before. A client gets `422 invalid-payload` or `422 invalid-state` where it got `202` or
`201` in two cases: what it sends serde accepts and the schema refuses (a field under its `#[serde(alias)]`, a form
that only a hand-written `Deserialize` takes), or it sent anything that does not match its schema to a node that does
not host the type. Before the nodes are upgraded:

1. Add `cicada_axum::assert_message_sample` and `assert_state_sample` to the tests of each type, with what the type
   serialises and with what clients send over HTTP ([The schema check](guide/http.md#the-schema-check)).
2. A sample that fails only the schema shows a type whose schema and serde disagree. When it is a form that only some
   clients send (an alias), stop sending it. When it is what the type itself writes (a field with `#[serde(with)]` and
   no `#[schemars(with)]`), the schema is wrong and every send of the typed client would be refused; there is no switch
   to turn the check off. Making the schema right changes the contract of the type: follow
   [Changing a contract](#changing-a-contract).

Clients need no upgrade.

**Version 0.13.0 changes no schema and no wire format.** It adds `cicada-app` (ADR 0027), so nodes of 0.12.0 and
0.13.0 can share a database. A host may move its start-up code onto `App`; nothing forces it. The command line moved
from `cicada-cli` into `cicada-app` and is still the binary `cicada`; install it with `--features tui` for the
terminal UI. A deployment of the sample service renames its variables ([Sample service](guide/app.md#sample-service));
a node of it refuses to start while one of the old ones is set.

**Version 0.14.0 changes no schema of the store, but it changes a wire format.** A `Failed` holds its cause (ADR
0028), so the contract of every type that watches changes, and `cicada-actors` replaces `cicada-extras`, whose dead
letters get a new contract. A system without a type that watches and without dead letters can deploy it rolling: nodes
of 0.13.0 and 0.14.0 share a database, and clients need no upgrade. A system with either cannot; follow the steps:

1. **The crate.** Replace `cicada-extras` by `cicada-actors` in the manifests. `Schedule` keeps its name
   (`cicada_actors::Schedule`). Drop `FailureWatcher` and the task that ran it: a watch replaces it (step 3).
2. **A type that watches.** Its contract changes with `Failed`. A `Failed` that a node of 0.13.0 wrote has no cause, and
   a watcher of 0.14.0 cannot decode it: that message ends `failed`, and a notification that fails tells nobody. So
   first let the watchers handle the notifications in their mailboxes (`GET /stats?prefix=<type>/` shows none
   pending), then follow [Changing a contract](#changing-a-contract) for each such type. A handler that builds a
   `Failed` by hand adds the cause.
3. **Dead letters.** Their contract changes too, and the letters of 0.13.0 are lost:
   1. Save the letters if they matter: `cicada get actor cicada.dead-letters/_ -o json`.
   2. Stop every node. While a node of 0.13.0 runs, its `FailureWatcher` creates the actor again after it is deleted.
   3. Start one node of 0.14.0 that does not host the type (`serve --types none`). Delete the actor and then the
      registration: `cicada delete actor cicada.dead-letters/_`, then `cicada delete type cicada.dead-letters`. A node
      that hosts the type refuses to start until that is done: its contract conflicts with the registered one.
   4. Start the other nodes, create the actor again (`DeadLetters::default()` with `DeadLettersState::default()`, or
      `cicada create actor cicada.dead-letters/_ --state '{}'`) and set its watch on every type: `ActorSystem::watch`, or
      `cicada create watch --target '*' --watcher cicada.dead-letters/_`.
4. **A supervisor and a topic** are new: nothing moves. Both watch, so create them only when no node of 0.13.0 is left:
   such a node writes a `Failed` without a cause. Host a supervisor on nodes that host the types of its children, so
   that a wrong entry is refused when the child starts ([Supervisor](guide/actors.md#supervisor)).

The sample service no longer reads `FAILURE_WATCHER_INTERVAL`: a node that hosts the dead letters creates them and their
watch at the start.

**Version 0.15.0 changes no schema of the store, but it changes the contracts of some types.** A message schema holds
every definition it refers to, whatever its name, and a reference to a family that holds itself accepts any value;
[The contract of a type](guide/core.md#the-contract-of-a-type) names the three kinds of type that are hit. The contract
hash of such a type changes, so a node of 0.15.0 that hosts it refuses to start against the registration of the
release before (`ContractConflict`). Every other type keeps its contract byte for byte: a system without a type that
is hit can deploy 0.15.0 rolling, with nodes of 0.14 and 0.15.0 on one database. Clients need no upgrade. Before any
node is deployed:

1. Raise the dependency to 0.15.0 and run the contract snapshot tests of every type (`assert_contracts_match`) against
   the snapshots of 0.14.0 (0.14.1 makes the same contracts). A snapshot that changes names a type that is hit.
2. For such a type, follow [Changing a contract](#changing-a-contract): a versioned type name, or a stop of the world
   that stops every node that hosts it, deletes its actors and then its registration (`DELETE /types/{name}` answers
   `409` while an actor of the type exists).

A type with a newtype variant of a type whose name a reference escapes could not start before 0.15.0, so nothing of it
is registered: it registers its first contract.

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
   the minor number while the version is 0.x, anything else the patch number;
   [the README](../README.md#using-a-release-in-another-project) says what counts as breaking. `docs/openapi.json`
   carries the version: regenerate it (a test says how when it is stale) and update the README's `tag = "vX.Y.Z"`
   lines.
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
  retention (7 days). Without `status` it lists every finished message. For longer, host `DeadLettersActor`
  (`cicada-actors`) and watch every type for it: failures become dead letters, each with its cause, that an operator
  redrives or discards with a message. A watcher of `*` is told of every failure without polling, in the transaction
  of the failure. A supervisor that gives up on a child fails a message of its own (`restart limit: …` with the cause
  `handler`, or `cannot start …` and `cannot read …` with the cause `refused`), so dead letters show it; a `Start` to
  the supervisor takes the child back.
- **Watches.** `GET /watches` (`cicada get watches`, `:watches` in `cicada tui`) lists them by target and watcher. A
  watcher of every type (`*`) gets a `Failed` in its mailbox for every failure of the system, and on PostgreSQL every
  commit that ends a message `failed` locks its row; failures are the rare case, and removals do not reach it, as a
  watch on a type tells of failures only. A watcher that does not keep up lets notifications pile up in its mailbox,
  which `GET /stats?prefix=` and its mailbox show. One event can write many messages, for an actor with many watchers
  or a tree whose actors are each watched; it is one transaction, like the removal of a tree. A watch on one actor
  stays when that actor is gone for good: a watcher that is done with a target removes its watch, for example when it
  handles the `Stopped`. A notification that fails tells nobody and is not retried; its outcome shows it, and
  `POST /messages/{id}/retry` sends it again.
- **A message that keeps killing its node** (out of memory, an abort, a native crash) ends `failed` at the claim limit,
  with the error `claim limit: <n> lost claims`; see [The claim limit](#the-claim-limit).
- **Stuck instances.** An instance whose state no longer decodes fails every turn: its messages end `failed` one by
  one. Fix the state type (add `#[serde(default)]`), or delete and re-create the instance (`DELETE`, then
  `PUT /actors/{type}/{key}`; the delete purges its mailbox and removes its descendants).
- **What a delete removes.** `DELETE /actors/{type}/{key}` (`cicada delete actor`) removes the actor and every
  descendant: the actors its turns created with `create_child`, theirs, and so on, in one transaction. Every pending
  message of them ends `purged`, a turn running on one commits nothing, and the delete counts the purged messages of
  the whole tree. Actors created with `create` are independent and stay. See what an actor owns first with
  `GET /actors/{type}/{key}/children` (`cicada get children`, or the `children` tab of an actor in `cicada tui`), one
  level at a time. A turn's `delete` removes the same, in the commit of that turn.
- **Control effects of turns.** A turn can pause, resume, purge and delete other actors (ADR 0024), for example a
  supervisor that restarts a worker with a delete and a create. Nothing in the store records who did it: the node logs
  each such effect of a commit it saw succeed (see [Logs](#logs)), and a watcher of the actor gets a `Stopped` with
  `by` for a delete. An effect on an actor that is not there does nothing, so a supervisor with a wrong address fails
  silently; the log line shows the target. Any actor can control any other.
- **What a restart does to a running handler.** A delete, from outside or by a turn, ends the lease of a turn running
  on the actor: that turn commits nothing, and its message ends `purged` and is never handled again. Its handler runs on
  until its next heartbeat finds the lease gone (within a third of the lease TTL) or, in blocking code, until it
  returns. When the actor is created again at once, as a restart does, the first turn of the new actor can start
  meanwhile, so for a moment two handlers run for one address: the old one commits nothing, but what it does outside
  the store overlaps with the new one. Use the message id as the idempotency key of external calls, and keep handlers
  that a supervisor may restart free of blocking code.
- **A large tree** is removed in one transaction that writes one `purged` outcome per pending message of every
  descendant; on SQLite it holds the single writer meanwhile. On PostgreSQL a removal that takes longer than
  `socket_timeout` (30 s) cannot finish: every delete of the tree answers `503 store-unavailable`, and a `stop()` of
  its root fails and is handled again and again. There are two ways out: remove the tree from the leaves up, listing
  each level with `GET /actors/{type}/{key}/children` and deleting the deepest actors first, or raise `socket_timeout`
  (`PostgresOptions`) for the node that runs the removal. A tree of many thousands of actors with full mailboxes is
  better removed bottom-up by its own code. A turn that deletes the root of a large tree is such a removal too, in its
  own commit, as a `stop()` is. One more reason: on PostgreSQL the watchers of the removed root are locked for the
  whole removal, so a turn running on one of them cannot renew its lease meanwhile; a removal that takes nearly as long
  as the lease costs that turn its lease, and its handler runs again.
- **Store down.** Nothing proceeds and HTTP answers 503 `store-unavailable`; `ActorClient` retries sends with the same
  id. Nodes recover on their own when the store returns: the dispatch loop retries claims after `poll_interval`, and
  the PostgreSQL wake-up listener reconnects with a back-off (from 250 ms, doubling up to 30 s),
  polling meanwhile.

### The claim limit

A message counts its lost claims, and a node fails a message with as many as its limit, `NodeConfig::max_lost_claims`
(5 by default), instead of running it (ADR 0025). The message ends `failed` with the error
`claim limit: <n> lost claims`, without asking its retry policy, and its watchers get a `Failed`.

- **What counts.** A claim is lost when another claim takes its place although its turn neither committed nor gave the
  lease back. That is a node that died or was killed hard, also as a matter of routine (a deploy that sends SIGKILL, an
  out-of-memory kill, a crash), a lease that ran out (a long pause of the process, a partition from the store), and a
  commit that did not get through. A node that stops in good order but leaves a lease to run out loses that claim
  too: a stop that comes after a handler ended, while its commit is under way, or that a handler which does not yield
  outlasts. So stop a node with its grace period (`shutdown_grace_period`), and give the limit room for routine kills,
  or raise it.
- **What does not count.** A release: a node that stops in good order and gives back the claims of handlers that had not
  ended, or a claim it could not run, such as one whose answer came from a slow store only after the lease it asked for
  had run out. A failed run that is committed as a retry. A lease that runs out with no claim
  after it: its turn can still commit.
- **One message takes others with it.** The limit cannot tell the message that kills a node from the messages that ran
  beside it: each of them loses its claim when the node dies. A message that kills its node at once, as often as the
  limit allows, takes every message that the node claimed with it each time to the limit too, and they all end `failed`
  together. Their errors name the limit: send them again (`POST /messages/{id}/retry`, `cicada retry`, or a bulk retry
  by prefix); the one that fails again is the cause. A supervisor that restarts an actor on every `Failed` restarts the
  actors of those messages too.
- **The count never goes back**, also not after a retry that was committed: it counts over the life of the message. A
  message that lives long and is retried often can gather lost claims over weeks, and the limit then ends it although
  its last loss is long ago. A message sent again is a new message with none.
- **How to see it.** `lostClaims` in the view of a message (`GET /messages/{id}`, the mailbox, `/outcomes`), the
  column `LOST` of `cicada get messages -o wide` and the line `Lost claims` of a message in `cicada tui`: a message on
  its way to the limit before it gets there. A node logs each failure by the limit at the warning level (see
  [Logs](#logs)). A watcher tells such a failure from a failure of a handler by the start of the error,
  `claim limit:`; it is a hint, as a handler can return an error with the same start.
- **Setting it.** `NodeConfig { max_lost_claims: Some(10), ..NodeConfig::default() }` raises it, `None` turns it off,
  and `Some(0)` fails the start. Each node applies its own limit, so set one value on every node of a system.

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
| error | `Turn of … panicked …`, `A turn task failed` | A panic outside the handler (the runtime): a bug. A handler's own panic is an ordinary handler failure (`panic: …`). |
| warn | `Message … has N lost claims, as many as the claim limit allows: it fails without running` | A message reached the [claim limit](#the-claim-limit): its turns kept dying, or ran beside one that did. |
| warn | `Claim of … for … arrived after its lease deadline; it is given back` | The store answered a claim so late that its lease had run out: nothing ran, and the claim is not lost. Often it means a slow or overloaded store. When it adds that giving it back failed, the lease runs out and the claim counts as lost. |
| warn | `An observer panicked when the turn started; it is left out of this turn: …`, `… when the turn ended: …`, `… when its turn was dropped: …` | An observer has a bug; the turn went on as if that observer were not there. |
| warn | `A header of an observer is left out of the sends of the turn: …`, `The header '…' of an observer is left out of a send to …` | A header that the limits of a message refuse, or that did not fit into a message of 64 headers; a trace breaks at that message. |
| warn | `Retention sweep failed`, `Store wake-ups ended; falling back to polling` | The node keeps working, slower or with more stored data. |
| warn | `Ignoring the retry policy of …` | A stored policy that cannot be decoded is read as no policy. |
| warn | `The schema of message type '…' of … cannot be used by the validator and is not enforced at the door: …`, `The schema of the state of … …` | The HTTP door cannot compile a registered schema (a `$ref` outside the document, a pattern its regular expressions do not know), so sends or creates of that type pass unchecked and a node that does not host it lets through what its decoder would refuse. Logged once per schema in the life of the process. The sample checks of the type fail too; fix its schema, which is a contract change. |
| info | `Cancelling N turns after the grace period of …` | A stop that cut running turns off. |
| info | `Turn of supervisor/main for 3f2c…: delete worker/w1` | A committed turn paused, resumed, purged or deleted an actor (fields `actor`, `effect`, `target`); whether the target existed, the commit does not say. Nothing else records who did it. |

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

Add `ActorTelemetry` (`cicada-otel`, [OpenTelemetry](guide/core.md#opentelemetry)) to a node with `observe` to get a
span per turn and the metrics `cicada.turn.duration` and `cicada.message.lag`. Useful signals:

- **Lag** (`cicada.message.lag`) rising for one type: too few nodes host it, `concurrency` is too low, or its turns
  got slower. A message is claimed at most `poll_interval` late when nodes are idle.
- **Failed turns** (`cicada.turn.duration` count with `cicada.outcome="failed"`), by type. Each such span has an
  `exception` event with the error, whatever failed: the handler, a timeout, a message or a state that does not decode,
  a rejected commit, or the claim limit (`claim limit: …`).
- **Retries** (`cicada.outcome="rescheduled"`) climbing: a dependency is failing and senders' retry policies are
  absorbing it. These spans have an `exception` event too.
- **Abandoned turns** (spans with `cicada.outcome="abandoned"`): turns of which the node saw no commit succeed. They
  lost their lease, were cut off by a stop, or their commit did not get through. They have no `exception` event, also
  when their handler failed; each one that another claim replaces is a lost claim of its message.
- Mailbox depth is not a metric (it needs a store query); `GET /stats?prefix=` counts the actors and pending messages
  under a prefix and names the delivery time of the oldest due message, and `GET /messages?prefix=` lists them. A
  call reads every row under the prefix, so poll it rarely, and with a prefix on a big store.
