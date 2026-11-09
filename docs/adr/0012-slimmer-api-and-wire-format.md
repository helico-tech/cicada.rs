# 0012. A slimmer API and wire format

Date: 2026-11-09

## Status

Accepted

## Context

A review on 2026-11-09 found features nothing used, places with two ways to do one thing, and leftovers of the Kotlin
port. The library should be complete, opinionated and not bloated. ADR 0008 allows free changes to the schema and the
wire format until there is a production database; there is none yet. The full list with its reasons is the change
`slim-the-library` in `openspec/`.

## Decision

**Plugins observe.** A plugin has three hooks: `on_claim`, `on_handler_error` and `on_outcome`. It never changes what
a turn commits; only the handler does. The one thing a plugin decides is to refuse a turn, by an error from
`on_claim`. Gone: `before_commit`, sends from hooks, replacing an outcome, reserved `cicada-` headers and installing
a plugin on one type. The one exception is `ClaimContext::send_headers`: in `on_claim` a plugin may add headers to
every message the turn sends; a send's own header of the same name wins. The OpenTelemetry plugin uses it, so trace
context travels without code in handlers, and `cicada_otel::trace_headers` is gone.

**One unhandled policy.** `Unhandled` is `Fail` or `Drop`. The custom policy (`on_unhandled`) is gone.

**One way to configure and send.** `NodeConfig` with `ActorSystemBuilder::config` replaces the builder's setters.
`SendOptions` is the only options type. A type's maximum turn is `Actor::MAX_TURN` or the node default.
`install` takes a plugin or an `Arc` of one.

**Wire format.**

- Durations are ISO-8601 strings only (`PT5S`). The short form (`5s`) and a number of milliseconds are not accepted,
  and a negative duration is refused (`400`), also as a send's `delay`.
- Retry and retention policies have one decoder, strict everywhere: an unknown key is an error in a request (`400`)
  and makes a stored policy read as none.
- Twelve endpoints. `GET /messages/{id}/chain` and `GET /types?uri=` are gone. Every message still carries
  `causedBy`.
- `params` of an actor is a plain JSON object; its keys are in alphabetical order, not template order.
- Length limits count characters, not UTF-16 units. Canonical JSON sorts object keys by their bytes, so a contract
  hash differs from before only for keys outside the basic multilingual plane.
- The problem type `invalid-headers` is gone.

**Stores.** The `Store` trait loses `retention()`, `type_def()`, `outcome()`, `resolve()`, `chain_entries()` and
`chain_children()`. Store settings nobody set are constants. `PostgresStore::connect(url)` hands the URL to sqlx;
`PostgresConnection` and `connect_as` are gone. The PostgreSQL migration lock key is computed from the schema name's
bytes.

**Extras.** `Timer` is gone: a timer is a delayed self-send whose id is kept in state and cancelled with
`turn.cancel`. `FailureWatcher` takes an `ActorSystem`; the `ActorOperations` trait and the `client` feature are gone.
Dead-letter message ids come from SHA-256.

**Kept on purpose.** The retention rule `Keep::Last` stays (decided 2026-11-09). So do `Schedule`, the dead-letter
blocks, `GET /messages?prefix=`, camelCase JSON, and everything named under "Kept on purpose" in the change.

## Consequences

- This is a breaking release, 0.2.0; `CHANGELOG.md` lists what callers must change.
- A failure watcher needs an `ActorSystem` in its process; a send-only node is enough.
- A message chain is no longer served in one call. A tool follows `causedBy` upwards itself; finding what a message
  caused needs a store query.
- Nodes of 0.1.0 and 0.2.0 must not share a database: the lock key and stored dead-letter ids differ, and 0.1.0 can
  write policies and durations that 0.2.0 refuses.
- Karakuri, the one planned user that was asked (2026-11-09), needs none of the removed plugin features. It asks that
  trace headers keep travelling to the messages a turn sends; that now happens automatically.
