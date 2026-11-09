# Changelog

Releases of Cicada for Rust. The crates share one version. They are used as git dependencies, so pin a tag or a
revision.

## 0.2.0 — unreleased

A breaking release that makes the library smaller (ADR 0011, ADR 0012). Nodes of 0.1.0 and 0.2.0 must not share a
database.

### Added

- `ClaimContext::send_headers`: a plugin's `on_claim` can add headers to every message the turn sends; `ActorTelemetry`
  uses it, so trace context reaches the messages a turn sends without code in the handler.

### Removed

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
- `PostgresConnection` and `PostgresStore::connect_as`. Store settings: `transaction_retries`, `connection_timeout`,
  `channel`, `min_idle`, `listener_reconnect_delay` (PostgreSQL) and `readers` (SQLite).
- `cicada-extras`: `Timer`, `ActorOperations`, `Failure`, `Sent`, `OperationError` and the `client` feature.
- `api::Params`, `From<MessageView> for Message`, `ActorSystem::wake_up`, `ActorTelemetry::current_trace_headers`
  (the free function `cicada_otel::current_trace_headers` stays), `cicada_otel::trace_headers` (now automatic),
  `time::SignedNanos`, `time::serde_signed_duration`.
- `sample-service`: `/console/`, `CONTAINERS`, `DATABASE_USER` and `DATABASE_PASSWORD`.

### Changed

- Plugin hooks take a shared context: `on_claim(&self, &ClaimContext)`, `on_handler_error(&self,
  &HandlerErrorContext)`. An `Arc` of a plugin is a plugin.
- Node settings: `ActorSystem::builder(store).config(NodeConfig { lease_ttl, ..NodeConfig::default() })`.
- Durations on the wire are ISO-8601 strings only (`"PT5S"`); `"5s"` and numbers of milliseconds are refused, and so
  is a negative duration. `time::parse_duration` returns a `Duration`.
- Retry and retention policies are decoded strictly everywhere; error texts changed.
- `ActorView::params` is a `BTreeMap`; its JSON keys are in alphabetical order.
- Length limits count characters; canonical JSON sorts keys by bytes.
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
