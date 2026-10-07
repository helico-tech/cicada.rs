---
status: done
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

- 2026-10-07: Wire model in `cicada::api` (`mod.rs` model, `mappings.rs` conversions, `strict.rs` strict request
  policies and lenient served ones). Responses write every field, nulls explicit, Kotlin field order. `retry` and
  `retention` of a send are validated by hand from JSON, so an unknown key at any depth is refused and duration errors
  name their field (`backoff.initial`, `delay`).
- 2026-10-07: `cicada-axum`: `actor_api`, `actor_api_with(ApiConfig)`, `actor_health`. One wrapper (`handlers::run`)
  renders `ApiProblem`, `OpError`, `StoreError` and panics (caught with `catch_unwind`, logged) as problem+json with
  the `OriginalUri` as `instance`, so it is right when nested. The actor path is parsed from the raw URI (mailbox
  detection, lenient `%XX` decoding, `.`/`..` as plain segments). Bodies: 413 by Content-Length and while streaming;
  a body must be a JSON object (serde's derived structs would also take an array of the fields).
- 2026-10-07: Tests: `api.rs` (ActorApiTest, 23), `robustness.rs` (ActorApiRobustnessTest, 18), `mounting.rs` (6:
  in-process SDK next to the API, non-hosting node, basic and bearer auth layers, prefix, public readiness, the
  SqliteNodeTest flow on the in-memory store with a restart), on virtual time with `oneshot`.
- 2026-10-07: Checked 21 recorded responses of every endpoint kind against the console's zod schemas
  (`actors-ui/src/main/web/src/api/generated.ts`) with a throwaway script: all parse, and every object has exactly the
  schema's keys in the schema's order.
- Not ported: `mounting the API without the plugin fails at start` and the plugin life-cycle and store-ownership
  tests of `ActorsPluginTest` (no plugin in Rust: the router takes the `ActorSystem` by value, and the host owns the
  store and calls `start`/`stop`); `stopping the application releases the leases` is runtime behaviour (story 07).

## Follow-ups

- Run the `SqliteNodeTest` flow (`mounting.rs`) on `SqliteStore` once story 05 lands.
- Story 10: turn the zod check into the fixtures test of ADR 0005.
- [0301](../../issues/0301-params-key-order.md): `params` key order differs from Kotlin.
