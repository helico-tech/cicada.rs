---
status: in-progress
created: 2026-10-07
---

# 08. HTTP API as axum routers with the Kotlin wire model and problem+json

## Goal

HTTP API as axum routers with the Kotlin wire model and problem+json.

## Acceptance criteria

- Fourteen endpoints with identical paths, bodies, statuses and problem types.
- `ActorApiTest` and `ActorApiRobustnessTest` ported.

## Plan

- Wire model: port `ApiModel.kt` and `ApiMappings.kt` to `cicada::api` (in the core crate, so server and client
  share it). Field names and order, explicit nulls, instants and durations as in `cicada::time`. Request envelopes
  decode strictly (unknown keys are errors, also inside `retry` and `retention`); payload and state stay lenient.
- Server: port `ActorApi.kt` to `cicada-axum`: `actor_api(system)` / `actor_api_with(system, ApiConfig)` and
  `actor_health(system, path)` as axum `Router`s without fallbacks, so hosts can `nest` or `merge` them and add their
  own auth layers. Handlers run inside one wrapper that renders problem+json (RFC 9457, `instance` = original request
  URI) for `ApiProblem`, `OpError`, `StoreError` and panics. Mailbox detection and address decoding on the raw path;
  body limit by Content-Length and while reading.
- Tests: port `ActorApiTest`, `ActorApiRobustnessTest` and the plugin-independent parts of `ActorsPluginTest` and
  `SqliteNodeTest` (on `InMemoryStore`) with `tower::ServiceExt::oneshot`, on virtual time.

## Progress log

- (none yet)

## Follow-ups

- (none yet)
