# Proposal

Status: decided by Arjan on 2026-11-09 (all yes, except C2); built on 2026-11-09. Tasks are in `tasks.md`. This change adds no
spec deltas (`skip_specs`): `docs/architecture.md` and the README stay the reference and are updated with it.

## Why

Cicada for Rust should be a durable actor library that is complete, opinionated and not bloated. A review on
2026-11-09 found features that nothing uses, several places with two ways to do one thing, and leftovers of the Kotlin
port. ADR 0008 says there are no production databases yet, so the schema and the wire format can still change freely.
ADR 0011 already removed `cicada-ui` and `cicada-docker`.

## What Changes

Decision column: `open` until Arjan answers, then `yes` or `no`. "No user" means: no caller outside the item's own
tests, checked with grep over `crates`, `examples`, `sample-service` and `e2e`.

### Cut

| # | Cut | Why | What it costs | Decision |
| --- | --- | --- | --- | --- |
| C1 | `Unhandled::Custom`, `Actor::on_unhandled`, `UnhandledDecision` | No user. It is a second handler path (`actor.rs:340-352`). `Fail` and `Drop` stay. | A type can no longer run code for a message it cannot decode. | yes (2026-11-09) |
| C2 | The retention rule `Keep::Last { count }` | No user. It costs two columns (`keep_last`, `keep_seq`) and an extra UPDATE per finished message in all three stores. `Remove` and `For` stay. | "Keep the last N records per actor" is gone. The columns are removed from schema version 1 in place (allowed by ADR 0008), not by a new migration. | no (2026-11-09) |
| C3 | `ActorOperations` trait and the `client` feature of `cicada-extras` | One real implementation. `FailureWatcher::new` takes an `ActorSystem`. About 170 lines and an optional dependency. | A failure watcher can no longer run in a process without an `ActorSystem`. | yes (2026-11-09) |
| C4 | `Timer` in `cicada-extras` (168 lines) | One user, an example. Other code keeps the message id in state and calls `turn.cancel`. One idiom is enough (see A2). | The Checkout example and its state sample change. | yes (2026-11-09) |
| C5 | Store trait methods `retention()`, `type_def()`, `outcome()`, `resolve()`; `chain_entries(ids)` becomes one id | Each is unused or the same as another method. 27 methods become 23. | Custom store backends (none known) must follow. Tests that hook `resolve` hook `list_types` instead. | yes (2026-11-09) |
| C6 | Store settings nobody sets become constants. PostgreSQL: `transaction_retries`, `connection_timeout`, `channel`, `min_idle`, `listener_reconnect_delay`. SQLite: `readers` | Real code sets only `schema` and `max_pool_size`. | Two tests that shorten a delay need another way to do it. | yes (2026-11-09) |
| C7 | `PostgresConnection` and `PostgresStore::connect_as` (236 lines) | A hand-written URL parser in front of sqlx's own. `connect(url)` and `connect_with(options)` stay. | `sample-service` loses `DATABASE_USER` and `DATABASE_PASSWORD`; credentials go in the URL. The change `redact-connect-options-url` is no longer needed. | yes (2026-11-09) |
| C8 | `GET /types?uri=`, `TypeMatchView`, `ActorClient::type_of` | `GET /actors/{path}` already returns type and params. The route returns two shapes today. | Thirteen endpoints instead of fourteen. You cannot ask the type of an address that has no actor. | yes (2026-11-09) |
| C9 | Public items with no user: `ActorSystem::wake_up`, `ActorSystem::hosted_templates`, `Page::map`, `From<MessageView> for Message`, `ActorTelemetry::current_trace_headers`, `Attributes::{remove, with_mut, contains}` | Dead surface. | None found. | yes (2026-11-09) |
| C10 | Duplicate tests: `cicada-store-sqlite/tests/differential.rs` (its header says it is a stopgap), client tests that re-test the server, e2e tests that re-test the HTTP contract, `cicada/tests/store_support.rs` | The same behaviour is tested three to four times. About 900 lines (estimate). | Less redundancy. The axum tests stay the home of the HTTP contract, the conformance suite the home of store behaviour. | yes (2026-11-09) |
| C11 | The changes `docker-events-watcher`, `console-container-logs`, `several-docker-hosts` | Their crates are gone (ADR 0011). | The text stays in git history as `work/issues/0800` to `0802`. | yes (2026-11-09) |
| C12 | Plugin surface: `before_commit`, sends from hooks, `set_outcome`, reserved `cicada-` headers, per-type installs | No user. See "Open questions". | Waits for karakuri's answer. | yes (2026-11-09) |
| C13 | Message chain lookup: `GET /messages/{id}/chain`, `ActorSystem::chain`, two store methods in three stores. `caused_by` stays on every message. | Only the console used it. | Waits for karakuri's answer. | yes (2026-11-09) |

### Harmonize

| # | Harmonize | From | To | Decision |
| --- | --- | --- | --- | --- |
| H1 | Durations on the wire | ISO-8601 (`PT5S`), short form (`5s`) or a number of milliseconds; negative allowed | ISO-8601 strings only; negative is refused. The short form stays only for the sample service's environment variables and moves there. | yes (2026-11-09) |
| H2 | Decoding retry and retention policies | Derived serde in `retry.rs` and `retention.rs`, plus a second hand-written decoder in `api/strict.rs:84-172` | The derived one only, with `deny_unknown_fields`. Error texts name the field less precisely. | yes (2026-11-09) |
| H3 | Kotlin-era encodings | Length limits count UTF-16 units; canonical JSON sorts keys by UTF-16 units; the migration lock key is a Java `hashCode`; error texts say `onClaim`, `beforeCommit` | Limits count characters; keys sort by bytes; a plain hash; Rust names. A contract hash changes only for keys outside the basic plane. | yes (2026-11-09) |
| H4 | Options of a send | `SendOptions` and `HookSend`, with the same six fields | `SendOptions` only | yes (2026-11-09) |
| H5 | Node settings | `.config(NodeConfig { .. })` and nine setters on the builder | `NodeConfig` and `.config()` only | yes (2026-11-09) |
| H6 | Maximum turn | `Actor::MAX_TURN`, `ActorType::max_turn()` (no user) and the node default | `Actor::MAX_TURN` and the node default | yes (2026-11-09) |
| H7 | Installing a plugin | `install` and `install_shared` | `install` only; an `Arc` of a plugin is a plugin | yes (2026-11-09) |
| H8 | Building a contract | `contract_of` repeats `Hosted::analyse` (`actor.rs:249` and `:376`) | One function | yes (2026-11-09) |
| H9 | Backend-neutral store logic | Paging, the message copy, the register decision and the claim guards are written in each of the three stores | Once, in `store::support` (about 70 lines less) | yes (2026-11-09) |
| H10 | Dead-letter message ids | A version-3 UUID through `md-5` and `uuid` | A SHA-256 id through `sha2`, which is already used. `md-5` leaves the workspace. | yes (2026-11-09) |
| H11 | `api::Params` | A 78-line ordered map, so JSON keys follow the template | `BTreeMap`; keys are alphabetical | yes (2026-11-09) |

### Add

| # | Add | Why | Decision |
| --- | --- | --- | --- |
| A1 | `CHANGELOG.md`, and version 0.2.0 for these breaking changes | Libraries others build on record their releases (Helico rule). There is no changelog yet. | yes (2026-11-09) |
| A2 | A README recipe for a timer: keep the message id in state, cancel it with `turn.cancel` | Replaces `Timer` if C4 is agreed. | yes (2026-11-09) |
| A3 | Whatever karakuri needs from plugins | See "Open questions". | yes (2026-11-09) |
| A4 | `ClaimContext::send_headers`: a plugin may add headers to a turn's sends; the OpenTelemetry plugin uses it and `cicada_otel::trace_headers` goes | Karakuri needs one trace per run across actors; with it, tracing needs no handler code. It is the one exception to "plugins never change what a turn commits". | yes (2026-11-09) |

### Kept on purpose

The turn executor and lease logic; the mandatory `UNHANDLED`; one-way sends; no jitter; separate SQL per backend; the
conformance suite; `InMemoryStore` and `TestActors`; contract snapshots and state samples; `cicada::api` in the core
crate; `cicada-client`; `cicada-otel`; `CrashLoopGuard`, `DeadLettersActor`, `FailureWatcher`, `Schedule`; the ten
examples; `GET /messages?prefix=`; camelCase JSON; the README as doctests.

## Open questions

- A3: the karakuri session was asked on 2026-11-09 what it needs from plugins (writes in the turn's commit, outcome
  changes, a veto, per-turn context, header propagation, chains). No answer yet. karakuri's architecture document says
  it keeps its state in plain tables, not in cicada.rs actors, so C12 and C13 go ahead; a wish that comes later becomes
  a change of its own.

## Out of scope

- A new console, or a new home for container handles.
- New features that nobody asked for.
- Changes to guarantees G1–G6 or store requirements S1–S10.

## Impact

- Public API of `cicada`, `cicada-extras`, `cicada-store-postgres`, `cicada-store-sqlite`, `cicada-axum`,
  `cicada-client`: breaking.
- Store schema version 1 (C2) and the HTTP wire format (C8, H1, H2, H11): need an ADR.
- `README.md`, `docs/architecture.md`, `docs/operations.md`.
