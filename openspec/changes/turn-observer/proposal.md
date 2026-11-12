# Proposal

## Why

The `Plugin` trait has three hooks, three context types and a typed key-value bag (`Attributes`), and a plugin may
refuse a turn at claim. It has two real users: the telemetry of `cicada-otel` and `CrashLoopGuard` in `cicada-extras`.
ADR 0018 replaces it with one narrow observer.

The guard worked by refusing a claim, which an observer cannot do. ADR 0018 leaves crash loops to this change: a
message that kills its node is claimed again for ever, and Chorus asks for a cap (its change `cicada-requests`,
point 6).

## What Changes

- **BREAKING** `Plugin`, `Attributes`, the three hook contexts, `HookResult`, `TurnInfo`,
  `ActorSystemBuilder::install` and `Turn::attributes` are removed. `HandlerFailure` leaves the public API.
- A new trait, `TurnObserver`, added to a node with `ActorSystemBuilder::observe`: it is called when a turn starts,
  may add headers to the messages the turn sends, and returns a value that is told the outcome when the turn ends. A
  value dropped without an outcome means the node saw no commit of the turn succeed. A node takes several
  observers.
- An observer cannot refuse a turn and cannot change what it commits. Its methods are plain functions, not async.
- `cicada-otel` implements `TurnObserver` and stays its own crate, so the core does not depend on OpenTelemetry. The
  core itself only writes `tracing` events. A turn that ends `failed` or is rescheduled now always has an
  `exception` event.
- **BREAKING** `CrashLoopGuard` and `CrashLoopError` of `cicada-extras` are removed. The core has a claim limit in
  their place, `NodeConfig::max_lost_claims`, 5 by default: a message with that many lost claims ends `failed` with
  the error `claim limit: <n> lost claims`, without running its handler, and a watch sees it. A claim is lost when
  another claim takes its place although its turn neither committed nor gave the lease back. A node that had no
  guard now has a limit. `NodeConfig` gets a public field.
- **BREAKING** A message counts its lost claims: `lost_claims`, a field of `Message` (code that builds one by hand
  adds it) and a column of both stores, and `lostClaims` in the view of a message over HTTP. The wide table of
  `cicada get messages` and the terminal UI show it. The claim that replaces a lease writes it.
- **BREAKING** Migration 6 is not additive: every node of 0.10.0 is stopped before the first node of 0.11.0 starts.
  No message starts with lost claims from before it.
- A header that an observer adds never fails a send of the handler: one that does not fit is left out, with a
  warning. A header of a plugin could fail a send.
- The statements of the PostgreSQL store list their columns, so that a later migration that only adds a column can
  be additive again.
- The sample service no longer reads `CRASH_LOOP_LIMIT`; it runs with the default limit.

The simplest version: one trait with one method and one returned value, and one number on the node.

## Capabilities

### New Capabilities

- `turn-observer`: code that watches turns start and end, without a say in them.
- `claim-limit`: a message whose turns keep dying ends `failed`.

### Modified Capabilities

- `turn-management`: who is told of a cancelled turn (an observer, not the hooks).
- `randomized-testing`: the comparison covers the lost claims, the simulation runs with a claim limit.
- `store-backends`: a store that is open keeps working when a column is added.

## Out of scope

- A metrics API in the core.
- Moving OpenTelemetry into the core.
- A claim limit per type or per message.
- Running a handler inside the trace context of an observer.

## Order

Step 7 of ADR 0018. After `control-effects`, before `json-schema-at-the-http-door`. It comes after `watches` so that
a message failed by the claim limit reaches a watcher.

## Impact

- `crates/cicada` (`plugin` becomes `observer`, `runtime`, `turn`, the store model and both stores, `api`),
  `cicada-otel`, `cicada-extras` (the guard is removed), `cicada-axum` and `cicada-client` (the view of a message),
  `cicada-cli`, the test plugins of `cicada` and `cicada-test`, the sample service, `e2e` (the test of a node that
  stops in good order can check that it lost no claim), `README.md`, `AGENTS.md` (its map names `plugin`),
  `docs/architecture.md` (the term *plugin*, turn semantics, G4, the schema paragraph, the crate table, how the
  runtime maps to tokio), `docs/operations.md` (telemetry, the claim limit, upgrading), `docs/openapi.json` and
  `CHANGELOG.md`.
- ADR 0018 supersedes the three plugin hooks of ADR 0012. ADR 0025 records the claim limit: it changes the wording
  of G4 and the wire format.
- Hosts: Chorus and Karakuri use neither the plugin trait nor the guard. They get the claim limit by default, and
  follow the new field of `Message` where they build one by hand.
