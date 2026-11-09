# Changelog

Releases of Cicada for Rust. The crates share one version. They are used as git dependencies, so pin a tag (README,
Using a release in another project).

Each section is headed `## X.Y.Z — YYYY-MM-DD`; the next version is `## X.Y.Z — unreleased` until it is released. While
the version is 0.x, a breaking change raises the minor number and anything else the patch number. A published tag never
moves: a correction is a new version.

## 0.3.0 — unreleased

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
