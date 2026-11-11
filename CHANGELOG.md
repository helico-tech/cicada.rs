# Changelog

Releases of Cicada for Rust. The crates share one version. They are used as git dependencies, so pin a tag (README,
Using a release in another project).

Each section is headed `## X.Y.Z — YYYY-MM-DD`; the next version is `## X.Y.Z — unreleased` until it is released. While
the version is 0.x, a breaking change raises the minor number and anything else the patch number. A published tag never
moves: a correction is a new version.

## 0.6.0 — 2026-11-11

A breaking release: a key is one name (ADR 0018, its third step; ADR 0021). A ref is written as one string, or is a
unit struct for a singleton; a `:` is an ordinary character of a name; contracts and the views of actors and types
hold no key fields. Every contract hash changes, and the store schema goes to version 3 with a migration that is not
additive. Nodes of 0.5.x and 0.6.0 must not share a database.

### Upgrading

1. A ref with two or more fields gets one string and joins its parts itself, with a separator its parts cannot hold. To
   keep reaching the actors that exist, join with `:` in the old order of the fields: the address `account/acme:7` of
   the old ref `{tenant: "acme", id: 7}` is the address of the name `acme:7`. A field that is not a string (a number, a
   boolean, a character, an enum variant) becomes one. A ref can also keep its typed fields and name `String` as its
   serde form (`#[serde(into = "String", try_from = "String")]`; README, Define an actor type). Such a ref is written
   differently inside states and messages, where refs travel as JSON (such as `reply_to: Order` in the saga example), so
   a state or a pending message from before no longer decodes and its turn ends `failed`. The plain way is to let its
   actors finish and drain their mailboxes with the old version. Where that is not possible, read the old form where the
   ref is held: `deserialize_with` on the field of the state or the message that holds the ref, not on the ref itself. A
   ref's one field cannot have a serde alias, and a ref cannot use a reader that takes more than one form
   (`deserialize_any`, `from` an untagged enum): the start check counts what the reader declares. A ref that keeps its
   one string field is not touched.
2. In the code, replace `ActorAddress::of(type, values)` with `ActorAddress::of(type, name)`, which takes one name,
   and `key_values()` with `name()`. Drop `to_params`, `from_params`, `RefBinding::key_fields`, the key argument of
   `Contract::new`, `Contract::key_fields`, `Contract::params_of`, `TypeView.key`, `ActorView.params` and
   `ActorSnapshot.params`. Contract snapshot files of an earlier version keep passing: a `key` entry is ignored.
   Then check every ref before the database is migrated: a ref that is not one string stops a node at start, which
   is after its store has migrated the database. Run the tests (a test on `TestActors` hosts the types and runs the
   same check), or start a node on a new database.
3. Upgrade every user of `cicada-client` together with the nodes: a client of 0.5.x cannot decode the view of an actor
   or a type from a node of 0.6.0, because its types require `params` and `key`.
4. With the old version, find the keys of two fields that become one name, such as (`a:b`, `c`) and (`a`, `b:c`),
   which both become `a:b:c`. This query lists them, on SQLite and on PostgreSQL (in the store's schema):

   ```sql
   SELECT replace(address, '%3A', ':') AS address, count(*) AS actors FROM actors
       GROUP BY replace(address, '%3A', ':') HAVING count(*) > 1;
   ```

   Delete one actor of each pair with the old version (`DELETE /actors/{type}/{key}`); an actor cannot be renamed.
   Otherwise the migration fails on the primary key of `actors` with the database's own error, and the database stays
   at version 2. Both stores report it as unavailable (`StoreError::Unavailable`). On SQLite the open fails and the
   node does not start; the sample node exits. On PostgreSQL a node that retries an unavailable store keeps retrying:
   the sample node logs "PostgreSQL unavailable" every second until its start timeout of 60 s. Outcomes are not
   checked: those of two actors deleted before merge under one address.
5. Stop every node of the old version: a node of 0.5.x does not open a database of version 3, and a running one
   refuses the contracts that the new nodes register.
6. Start the new version: the nodes that host a type before the nodes whose turns create actors of it, and nodes that
   only send last. The first node migrates the schema to version 3; each node registers its types. Migration 3
   removes every registration, so until a node that hosts a type starts, that type is not registered: its actors can
   be read and deleted, and messages already in a mailbox wait; a create of one and a send to one from outside a turn
   are refused (over HTTP `404`), and a turn on another node that creates one ends `failed` without a retry, and a
   raw send to one in a turn is refused. A host whose nodes all host the same types has no such window. Where two
   kinds of nodes create actors of each other's types, no order is safe: hold back the messages that start such turns
   until every node is up.

Or start on a new database; then steps 1 to 3 are left. There is no way back: 0.5.x does not open a migrated
database.

A listing prefix that holds the encoded colon itself stops matching, because the stored spelling of a colon is `:`
now: `x%3A` given to `ActorSystem::list_actors` (or another listing) in code, or `%253A` in a URL. Over HTTP a query
value is URL-decoded first, so `?prefix=x%3A` arrives as `x:` and still matches. Write `:`.

### Changed (breaking)

- A ref holds one name. What counts is what serde does with it: a struct of which serde writes one field, whose value is
  a string (a `String`, a newtype around one, a `Uuid`; a field with `#[serde(skip)]` does not count, so such a
  `PhantomData` is fine); a type that serde writes as a string itself (a newtype around a `String`, or one that names
  `String` as its serde form); or a unit struct or a struct that declares no field to serde, a singleton with the key
  `_`, as before. A ref that declares a field and writes none (`skip_serializing`, or `skip_serializing_if` when it
  skips) has no address. Only a string counts: a number, a boolean, a single character and an enum variant are refused,
  although the last two worked as a key field before. A ref's one field cannot have a serde alias, and a ref cannot use
  a reader that takes more than one form (`deserialize_any`, `from` an untagged enum): the start check counts what the
  reader declares. A node that hosts a type whose ref is not one string does not start, and the error names the ref and
  what its `Deserialize` asks for. `ActorRef::address` fails for a ref that serde does not write as one string, so a
  send to it or a create of it fails at the call (`TurnError::Invalid`) and the turn can go on.
- A name may hold any text and must not be empty. In an address it is percent-encoded as before, except that `:` is
  now written as it is: `percent::encode` keeps `A-Z a-z 0-9 . _ ~ -` and `:`. A `:` separates nothing, so an old
  address such as `account/acme:7` stays valid as the one name `acme:7`, and `ActorAddress::parse` reads `a%3Ab` as
  `a:b`. The keys `a:`, `:a` and `a::b` are valid now.
- `ActorAddress::of(type_name, name)` takes one name; the singleton is the name `ActorAddress::SINGLETON_KEY`.
  `ActorAddress::name()` gives the decoded name and replaces `key_values()`.
- `to_params`, `from_params` and the serde bridge behind them (`address/params.rs`) are removed; a small reader takes
  the one string out of a ref and puts it back. `RefBinding` holds no list of fields: `key_fields()` is removed, and
  `RefBinding::new` checks the ref by what its `Deserialize` asks for.
- A contract no longer lists key fields: not in `Contract::new`, not in its JSON form (`type`, `state` and
  `messages`), not in its hash. Every contract hash changes; renaming the one field of a ref keeps the hash.
  `Contract::key_fields` and `Contract::params_of` are removed. `Contract::from_json` ignores a `key` entry, so a
  contract of an earlier version reads as the same contract.
- `TypeView.key` (`GET /types`, `GET /types/{name}`), `ActorView.params` (`GET /actors/{type}/{key}`) and
  `ActorSnapshot.params` leave the wire format and the API; `docs/openapi.json` follows. The command line loses the
  `KEY` column of `cicada get types`, and the terminal UI the `Key` facts of an actor and a type.
- A node that hosts a type no longer refuses an address because it is not the spelling its ref writes (`x/007` for an
  integer): there is one spelling per name. It still decodes the address as its ref and refuses one the ref does not
  accept (`422 invalid-address`): a singleton accepts only `_`, and a ref with a reader of its own may refuse a name.
  A typed send to a turn's own ref goes to the address that ref writes, no longer always to the claimed address.
- Schema version 3 (both stores): migration 3 removes every registration and rewrites `%3A` to `:` in
  `actors.address`, `messages.actor_address` and `outcomes.actor_address`, only in rows that hold `%3A`. The sender
  of a message and the addresses inside a stored envelope are read through `ActorAddress::parse`, which gives the new
  spelling. It is not additive (see Upgrading).

## 0.5.1 — 2026-11-11

No change to the wire format or the store schema: nodes of 0.5.0 and 0.5.1 can share a database.

### Fixed

- A store on PostgreSQL opens over TLS. The feature `postgres` builds the driver with TLS: rustls on the crypto
  provider `aws-lc-rs`, the one of `cicada-client`, trusting the built-in list of public CAs. A URL with
  `sslmode=require`, `verify-ca` or `verify-full` no longer fails with "SQLx was built without TLS support enabled";
  `sslrootcert` names the file of a private or cloud-provider CA, and `sslcert` and `sslkey` those of a client
  certificate. Without `sslmode` in the URL the driver takes `PGSSLMODE`, and without that the default `prefer`,
  which now means TLS without checking the server's certificate when the server offers TLS, and plain text when it
  does not; once the server has offered TLS, a failed handshake fails the connection, and `sslmode=disable` is the
  way back to plain text. Set the mode on purpose (docs/operations.md, TLS for PostgreSQL). A host that adds a
  library that builds rustls on `ring` must install a default crypto provider at start. A host with the feature
  `postgres` now compiles a C library (`aws-lc-sys`), so its build needs a C compiler. The feature `sqlite` builds no
  TLS.

## 0.5.0 — 2026-11-11

A breaking release: the store trait leaves the public API (ADR 0018, its second step). A host opens one public type,
`Store`, and the two SQL stores move into `cicada`, each behind a cargo feature. Nothing changes in the database: no
migration, and nodes of 0.4.0 and 0.5.0 can share one.

### Upgrading

1. `Cargo.toml`: drop `cicada-store-sqlite` or `cicada-store-postgres`, and name the feature on `cicada`:
   `cicada = { git = "…", tag = "v0.5.0", features = ["sqlite"] }` (or `["postgres"]`, or both). `cicada-test` stays a
   dev-dependency.
2. Open the store with `Store::sqlite(path, options)` or `Store::postgres(url, options)` (`cicada::Store`), and pass it
   to `ActorSystem::builder`. Replace `Arc<dyn Store>` with `Store`, and `PostgresStoreConfig` with `PostgresOptions`
   (`cicada::store::PostgresOptions`; `SqliteOptions` is `cicada::store::SqliteOptions`).
3. Replace calls on the store with calls on the system: `store.now()` with `system.now()`, `store.next_due_at(&types)`
   with `system.next_due_at()` (it takes no types: it answers for the types the node hosts, and `None` when it hosts
   none), and `store.running_turn(…)` and `store.pending(…)` with the methods of the same name. What differs: every
   call on the system returns `OpError`, not `StoreError` (a store failure is `OpError::Store`), and `system.pending`
   answers an `Option` of a page, `None` when the actor does not exist, where the store answered an empty page.
   `system.now()`, `system.next_due_at()` and `system.running_turn(…)` answer the same values as the store did.
   `hash_conflict_reason` is now `cicada::store::hash_conflict_reason`.
4. Replace `system.store().close()` with `store.close()` on the store the host opened.
5. In tests: `TestActors` is unchanged. Replace `InMemoryStore::default()`, `with_clock(…)` and `new(…)` with
   `Store::memory()`, `Store::memory_on(…)` and `Store::memory_with(…)`; read a state with `system.get_actor(…)`
   instead of `store.get(…)`; replace a `store.claim_due(…)` that plays a crashed node with `claim_and_abandon`.

### Changed (breaking)

- The `Store` trait is private to `cicada` (named `Backend` inside it), and so are the types only a backend needs:
  `CommitRequest`, `CommitResult`, `Claim`, `ActorCreate`, `StateChange`, `OutgoingMessage`, `NodeRecord`, the results
  of store-only calls (`CreateResult`, `EnqueueResult`, `RegisterResult`, `RenewResult`), the schema modules of the
  stores (`schema::VERSION`), `is_listening` and `cicada::store::support`. The compiler enforces it: code outside the
  core can neither implement a store nor call one.
- `Store` (`cicada::Store`, also `cicada::store::Store`) is a public type that a host cannot look into: it holds one
  backend and is cheap to clone, and its clones share the backend. `Store::sqlite(path, options)` (feature `sqlite`)
  and `Store::postgres(url, options)` (feature `postgres`) open one; `close()` releases it for every clone.
  `ActorSystem::builder` takes a `Store`.
- `ActorSystem::store()` is removed. The host closes the store it opened; after `close` every operation fails as
  unavailable and HTTP answers `503 store-unavailable`.
- The SQLite and PostgreSQL stores live in `cicada`, behind the features `sqlite` and `postgres`. No feature is on by
  default: without one the crate builds and opens no store. The crates `cicada-store-sqlite` and
  `cicada-store-postgres` are removed, and with them the names `SqliteStore` and `PostgresStore`.
- PostgreSQL is opened from a URL only: `connect_with` and the re-exported sqlx types (`PgConnectOptions`,
  `PgSslMode`) are removed, so no type of the SQL driver is in the public API. `PostgresStoreConfig` is renamed
  `PostgresOptions` and keeps its fields.
- The in-memory store, the test clocks and `TestActors` live in `cicada::testing`, behind the feature `testing`, which
  `cicada-test` turns on. `cicada-test` exports them under the names they had, so a test on `TestActors` changes no
  import. The name `InMemoryStore` is removed: `Store::memory()`, `Store::memory_on(clock)` and
  `Store::memory_with(clock, retention)` open the in-memory store. `TestActors::store()` returns the `Store`.
- The crates `cicada-store-conformance` and `cicada-fuzz` are removed. The conformance suite, the randomised tests and
  the runtime tests that need the store trait are test code of `cicada`: `cargo test -p cicada` runs the conformance
  cases for the in-memory store and SQLite, and for PostgreSQL when `CICADA_PG_URL` is set. A long randomised run is
  `PROPTEST_CASES=100000 cargo test -p cicada --lib tests::fuzz`.

### Added

- `ActorSystem::now()`, the store's time, and `ActorSystem::next_due_at()`, the earliest time at which a message of a
  type the node hosts may become claimable (paused actors left out).
- `claim_and_abandon(store, types, lease, limit)` (`cicada::testing`, exported by `cicada-test`) plays a node that claims
  a turn and dies: it claims what is due as a node of its own, commits nothing and returns an `AbandonedClaim` per
  claim, so the message is claimed again when the lease runs out.
- `cicada::store::hash_conflict_reason`, the one function of the old `store::support` that stays public.
- A conformance case pins that every operation of a closed store fails as unavailable, on every backend.

## 0.4.0 — 2026-11-11

A breaking release: every actor has state (ADR 0018, its first step). The store schema goes to version 2 with a
migration that is not additive, and the contract of every type that was stateless gets a new hash. Nodes of 0.3.x and
0.4.0 must not share a database.

### Upgrading

1. In the code, replace `Stateless` with `NoState` and `create_stateless(&r)` with `create(&r, &NoState {})` (on
   `Turn`, `ActorSystem`, `ActorClient` and `TestActors`), and give every create over HTTP a `state`:
   `{"state": {}}` for a `NoState` type, `cicada create actor ADDR --state '{}'` on the command line.
2. Delete the contract snapshot files of the formerly stateless types; the next test run writes them again. A snapshot
   with `"state": null` is no longer a contract, so `assert_contracts_match` fails on it, also with
   `CICADA_UPDATE_CONTRACTS=true`.
3. A type whose state encodes to `null` (`()`, a unit struct, a top-level `Option`) cannot be upgraded in place:
   change its state to a struct and, with the old version, reset the type (delete its actors, then
   `DELETE /types/{name}`), as for any contract change.
4. Stop every node of the old version: an old node that writes an actor without a state fails on the new schema.
5. Start the new version, the nodes that host the formerly stateless types first. The first node migrates the schema
   to version 2, and each node registers its types. Until a node that hosts a formerly stateless type starts, that
   type is not registered: its actors can be read and deleted, and messages already in a mailbox wait; a create of
   one and a send to one from outside a turn are refused (over HTTP `404`), and a turn on another node that creates
   one ends `failed` without a retry.

Or start on a new database; then steps 1, 2 and 4 are left, and of step 3 only the change of the state to a struct
(no reset). There is no way back: 0.3.x does not open a migrated database.

### Changed (breaking)

- `Stateless`, the `ActorState` trait and `create_stateless` (on `Turn`, `ActorSystem`, `ActorClient` and
  `TestActors`) are removed. `Actor::State` is any `Serialize + DeserializeOwned + JsonSchema + Send + Sync` type.
- `NoState` (`cicada::NoState`, also in the prelude) is the state of a type that keeps no data: an empty struct, stored
  as `{}`. It refuses unknown fields, so a node that hosts the type refuses a state such as `{"oops": 1}`.
- State is never absent. In the store the column is `NOT NULL`; `Claim.state`, `ActorRecord.state`,
  `ActorCreate.state` and `StateChange::NewState` hold a `Value`, and `Store::create` takes one. A contract always has
  a state schema: `Contract::new` and `Contract::state_schema` use a `Value`, and `Contract::from_json` refuses a
  contract whose `state` is absent or `null`. On the wire `ActorView.state`, `TypeView.state` and
  `CreateActorRequest.state` are a `Value` and always written; `ActorSnapshot.state` is a `Value` too, and
  `CreateActorRequest` no longer implements `Default`.
- Schema version 2 (both stores): migration 2 gives every actor without a state, or with the state `null`, the state
  `{}`, makes the column `NOT NULL` and removes the registration of every type without a state schema, so that a node
  that hosts such a type registers it again at start. It is not additive (see Upgrading).
- `PUT /actors/{type}/{key}` needs a state: an empty body or a body without `state` is a `400`.
  `ActorSystem::create_actor` and `ActorClient::create_actor` take a `Value`. `cicada create actor` needs `--state`.
- A state that is JSON `null` is refused everywhere: `Turn::create` returns `TurnError::Invalid`,
  `ActorSystem::create_actor` returns `CreateActorResult::InvalidState` on every node (`422 invalid-state` over HTTP),
  and a turn that ends with a `null` state ends its message `failed`, without a retry, and keeps the stored state.
- The contract hash of every type that was stateless changes, because its contract gains a state schema.
- A turn of a `NoState` type that takes `state_mut()` or calls `set_state` writes the state and increases the version,
  as for any other type.
- The command line's types table drops the column that said whether a type has state, and the terminal UI drops the
  "Stateful" fact of a type. `describe_changes` (`cicada-test`) reports any change of the state schema as "state
  schema changed".

## 0.3.2 — 2026-11-11

No library crate changed, only the test crate `cicada-fuzz`: nodes of 0.3.1 and 0.3.2 can share a database.

### Fixed

- The backend comparison of `cicada-fuzz` no longer depends on the order in which a store hands out epochs to
  different instances. When two instances are due at the same instant, `InMemoryStore` and `SqliteStore` may give
  out their epochs in another order (S4 promises an order per address only), and a stale or future epoch
  (`epoch_delta`) then reached different claims in the two. It now moves by rank among the epochs handed to the same
  address. The list that failed a CI run of 0.3.1 is now a test of its own.

## 0.3.1 — 2026-11-11

No change to the wire format or the store schema: nodes of 0.3.0 and 0.3.1 can share a database.

### Fixed

- A duration whose parts sum out of range is a parse error, also over HTTP (the `delay` of a send, a retry or
  retention policy). Before, a debug build panicked and a release build let the sum wrap, so a crafted duration could
  be accepted as a small one.

### Added

- `cicada-fuzz`, a test-only crate of randomised tests: the parsers, a comparison of the store backends, and a node
  simulation under injected store faults. A short run is part of `cargo test --workspace`;
  `PROPTEST_CASES=100000 cargo test -p cicada-fuzz` is a long one. For repeatable runs, the three `tokio::select!`
  calls of the runtime are now `biased`.

- `cicada-cli`, the binary `cicada`: a command line and terminal UI for any node that serves `actor_api()`
  (`actor_api_read()` for the commands that only read). It talks to the node only over HTTP, through `cicada-client`.
  - kubectl-style commands, verb first: `get`, `describe`, `create`, `delete`, `send`, `cancel`, `retry`,
    `deliver-now`, `pause`, `resume`, `purge`, `stats`, `health`. Output as a table, `-o wide`, `-o json` or
    `-o yaml`; `--all` follows the cursor to the last page. A missing item or an error response is exit code 1.
  - Named contexts in `~/.config/cicada/config.toml` (or `$XDG_CONFIG_HOME`, or `$CICADA_CONFIG`), each with a server,
    an optional bearer token and headers; `cicada config get-contexts`, `current-context`, `use-context` and
    `set-context`. `--server`, `--token` and `--context` win over `CICADA_SERVER`, `CICADA_TOKEN` and
    `CICADA_CONTEXT`, which win over the file. On Unix the file has mode 0600 after every write.
  - `cicada tui`: a k9s-style terminal UI. Actors, pending messages, outcomes, running turns, nodes and types,
    refreshed every 2 s; overviews of an actor, message, node or type; a live filter, sorting and the mouse; and
    delete, cancel, retry, deliver-now, pause, resume, purge and stop-turn behind a confirmation.

## 0.3.0 — 2026-11-09

A breaking release (ADR 0017). Every contract hash changes, so nodes of 0.2.0 and 0.3.0 must not share a database: a
0.3.0 node refuses to start on a store where a 0.2.0 node registered its types.

### Changed

- A contract hash leaves out the `description` keywords of the state and message schemas, at any depth. A reworded doc
  comment no longer changes the hash or stops nodes from starting. A property, definition or message type named
  `description` still counts. The published contract (`Contract::to_json`, `GET /types`) keeps its descriptions;
  after a doc comment change it keeps those of the first registration.
- `SqliteStore::open` refuses a database that has no schema version but holds tables, with `StoreError::Invalid` naming
  the path, and leaves the file as it was. A new or empty file is still set up. WAL mode is now set after the
  migration.

## 0.2.0 — 2026-11-09

A breaking release that makes the library smaller (ADR 0011, ADR 0012, ADR 0013) and adds management endpoints (ADR 0014). Nodes of 0.1.0 and 0.2.0 must not share a
database.

### Added

- `actor_api_read`: only the routes that read. Both routers serve `GET /openapi.json`, an OpenAPI 3.1 document, with a
  copy in `docs/openapi.json`. `GET /types/{name}` and `ActorClient::get_type`.
- `ClaimContext::send_headers`: a plugin's `on_claim` can add headers to every message the turn sends; `ActorTelemetry`
  uses it, so trace context reaches the messages a turn sends without code in the handler.

- Management reads and writes over HTTP (ADR 0014): `GET /outcomes?status=&prefix=&since=` (finished messages of any
  status), `GET /stats?prefix=` (`StatsView`: actors, pending messages, oldest due), `GET /messages/{id}/caused` (what
  a message caused, one level down) and `DELETE /types/{name}` (problem type `type-in-use`). In code:
  `Store::outcomes`, `Store::stats`, `Store::caused_by`, `Store::remove_type` (`StoreStats`, `RemoveTypeResult`),
  and `ActorSystem` and `ActorClient` methods of the same names. The schema keeps `caused_by` in a column of
  `outcomes` and indexes it, with `messages.caused_by`.

- Operator actions (ADR 0015). Nodes: `nodes` table written by a 10 s heartbeat of every node, `GET /nodes`,
  `GET /nodes/{node}` (problem type `no-such-node`); `Store::heartbeat_node`, `list_nodes`, `remove_node`;
  `ActorSystem::nodes` and `node`. Running turns: `GET /turns`, `GET /actors/{type}/{key}/turn` (problem type
  `no-running-turn`) and `POST /actors/{type}/{key}/turn/cancel`: the owning node stops the turn at its next heartbeat
  and commits the new `TurnOutcome::Cancelled` (`Store::running_turns`, `running_turn`, `request_cancel`,
  `RenewResult::CancelRequested`, `actors.cancel_epoch`). Mailboxes: `GET /actors/{type}/{key}/mailbox`, with
  `state` per message; `POST .../mailbox/pause`, `/resume` and `/purge` (`Store::set_paused`, `purge`,
  `actors.paused`, `ActorView::paused`). Messages: `POST /messages/{id}/retry` (problem type `message-not-retryable`),
  `POST /messages/{id}/deliver-now` (`Store::deliver_now`), `POST /messages/bulk-cancel` and `/bulk-retry`. Schema
  version 1 gets the table, columns and the partial indexes `actors_due` (now also without paused actors) and
  `actors_running`.

### Changed

- `Store::create` answers `CreateResult::NoSuchType` when the type is not registered, and a commit whose creates name
  an unregistered type is `CommitResult::Rejected`: the store refuses atomically. `ActorSystem::create_actor` no longer
  looks the type up again after the insert. Store implementations and fixtures must register a type before creating an
  instance of it.
- `POST /messages/{id}/cancel` replaces `DELETE /messages/{id}` (same semantics); `GET /actors/{type}/{key}/mailbox`
  replaces `GET /actors/{type}/{key}/messages`; `POST .../messages` stays as send. `ActorClient::pending` is now
  `ActorClient::mailbox`.

### Removed

- `GET /failures`, `Store::failures`, `ActorSystem::failures` and `ActorClient::failures`: use `outcomes` with
  `OutcomeStatus::Failed`.
- The crates `cicada-ui` (the console) and `cicada-docker` (container handles). Their last version is commit `7d43cf0`.
- `Unhandled::Custom`, `Actor::on_unhandled` and `UnhandledDecision`. Use `Unhandled::Fail` or `Unhandled::Drop`.
- Plugins: the `before_commit` hook, `HookSend` and sends from hooks, `HandlerErrorContext::set_outcome`, `outcome`
  and `failure_reason`, reserved `cicada-` headers (`SendResult::InvalidHeaders`, problem type `invalid-headers`), and
  plugins on one type (`ActorType::install`). `Attributes::remove`, `with_mut` and `contains`.
- `ActorSystemBuilder`: `install_shared` and the setters `node_id`, `lease_ttl`, `heartbeat_interval`,
  `poll_interval`, `default_max_turn`, `concurrency`, `shutdown_grace_period`, `shutdown_cancel_timeout` and
  `sweep_interval`. `ActorType::max_turn` and `ActorType::install_shared`.
- Message chains: `GET /messages/{id}/chain`, `ActorSystem::chain`, `ActorClient::chain`, `MessageChain`,
  `MessageChainView`, `ChainNodeView`. `causedBy` stays on every message.
- `GET /types?uri=`, `ActorSystem::type_of`, `ActorClient::type_of`, `TypeMatchView`, `ResolvedActorType`.
- `Store`: `retention()`, `type_def()`, `outcome()`, `resolve()`, `chain_entries()`, `chain_children()`; `ChainEntry`;
  `Page::map`.
- `UriTemplate`, overlap checks between types, `support::resolve`, `ResolvedType`, `ActorAddress::to_http_path` and
  `from_http_path`, `ActorSystem::hosted_templates` (now `hosted_types`).
- `PostgresConnection` and `PostgresStore::connect_as`. Store settings: `transaction_retries`, `connection_timeout`,
  `channel`, `min_idle`, `listener_reconnect_delay` (PostgreSQL) and `readers` (SQLite).
- `cicada-extras`: `Timer`, `ActorOperations`, `Failure`, `Sent`, `OperationError` and the `client` feature.
- `api::Params`, `From<MessageView> for Message`, `ActorSystem::wake_up`, `ActorTelemetry::current_trace_headers`
  (the free function `cicada_otel::current_trace_headers` stays), `cicada_otel::trace_headers` (now automatic),
  `time::SignedNanos`, `time::serde_signed_duration`.
- `sample-service`: `/console/`, `CONTAINERS`, `DATABASE_USER` and `DATABASE_PASSWORD`.

### Changed

- Actors are addressed by type name and key instead of URIs (ADR 0013): `ActorRef` has `const TYPE: &'static str = "sync"`
  instead of `const TEMPLATE`, and the address is `sync/acme`. Several ref fields form the key `b1:7`; a ref without
  fields is the singleton `pricing/_`. `TypeName` replaces `UriTemplate`.
- HTTP paths are `/actors/{type}/{key}` and `/actors/{type}/{key}/messages`. `ActorView` has `address` and `type`, and
  `TypeView` has `name` and `key`. `prefix` is a prefix of the address, without a scheme.
- `Store::create` has no template argument; `ActorRecord`, `Claim` and `ActorCreate` have no template field, and
  `TypeDef.template` is `TypeDef.name`. Every contract hash changes.
- Plugin hooks take a shared context: `on_claim(&self, &ClaimContext)`, `on_handler_error(&self,
  &HandlerErrorContext)`. An `Arc` of a plugin is a plugin.
- Node settings: `ActorSystem::builder(store).config(NodeConfig { lease_ttl, ..NodeConfig::default() })`.
- Durations on the wire are ISO-8601 strings only (`"PT5S"`); `"5s"` and numbers of milliseconds are refused, and so
  is a negative duration. `time::parse_duration` returns a `Duration`.
- Retry and retention policies are decoded strictly everywhere; error texts changed.
- `ActorView::params` is a `BTreeMap`; its JSON keys are in alphabetical order.
- Length limits count characters; canonical JSON sorts keys by bytes.
- `FailureWatcher` reads `outcomes` with the status `failed`.
- `FailureWatcher::new` takes an `ActorSystem`. Dead-letter message ids are `dead-letter-<sha256>`.
- Error texts name hooks as `on_claim`, `on_handler_error`, `on_outcome`.
- The HTTP API has twelve endpoints.

### How to move

- A timer: keep the id that `turn.send_self_with(..)` returns in state and cancel it with `turn.cancel(&id)`
  (README, Extras).
- A plugin that sent messages or changed an outcome: do it in the handler.
- PostgreSQL credentials: put them in the URL, or build `PgConnectOptions` and call `connect_with`.

## 0.1.0

The first version: the port of the Kotlin library (ADR 0001 to 0009).
