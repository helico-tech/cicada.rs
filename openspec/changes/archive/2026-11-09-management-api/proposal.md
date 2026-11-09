# Proposal

Status: agreed by Arjan on 2026-11-09; built on 2026-11-09. Depends on
`actor-type-and-key`.

The separate actions are their own changes: `node-view`, `turn-management`, `mailbox-management`, `message-actions`, `bulk-message-actions`, `outcomes-listing`, `remove-registered-type`,
`store-counts`, `messages-by-cause`. This change is the base they add routes to.

## Why

The HTTP API should be a proper introspection and management API, and clients should be able to be generated from a
published description. Today nothing describes it except prose in the README, and read and write routes cannot be
mounted apart.

## What Changes

- **S1. Two routers.** The new API replaces today's. `actor_api_read(system)` has no write routes at all;
  `actor_api(system)` has everything. Both stay plain axum routers that the host nests and wraps in its own
  authentication. A separate read router is safer than a per-action check: a missing check in Oban Web's read-only mode
  became CVE-2026-48592.
- **S3. OpenAPI document.** `GET /openapi.json` serves it, generated from the routes and wire types. A copy is committed
  and a test fails when the two differ.
- **S4. Payloads are free-form JSON** in the document. Each type's schemas stay available from `/types/{name}`, because
  they are known only at run time.
- **S5. `cicada-client` stays hand-written** and is tested against the document. Progenitor, the main Rust generator,
  reads OpenAPI 3.0 only; its 3.1 issues are open. Other languages can generate from the document.
- **I1. What carries over:** actors (list, get, create, delete), send, `GET /messages`, `GET /messages/{id}`,
  `GET /types`, `GET /types/{name}`, `GET /health`, RFC 9457 errors.
- **Tool:** `aide`, pinned to `=0.16.0-alpha.4`. Tried on 2026-11-09: aide 0.15.1 pulls in schemars 0.9 and fails to
  compile with our schemars 1 types; the alpha works with axum 0.8.9 and schemars 1.2.3 and produced a correct document.
  It is an alpha, and the last stable release was August 2025. `utoipa` is the alternative: more widely used, but it
  needs a second derive on every wire type. `poem-openapi` and `dropshot` are their own server frameworks.

## Out of scope

- Editing actor state by hand, reordering messages, remote control of nodes, cluster leader fields, forcing a lease away
  from a running node.
- A console, or generated clients in this repo.
- Version prefixes in paths; the host chooses where it mounts the routers.

## Impact

- Breaking for HTTP callers and for `actor_api` users. Changes the wire format, so it needs an ADR.
- `crates/cicada/src/api` (wire types derive `JsonSchema`), `cicada-axum`, `cicada-client`, `sample-service`, `e2e`,
  CI.
- A new dependency on an alpha release in `cicada-axum`.
- `README.md`, `docs/architecture.md`, `docs/operations.md`.
- Not yet tested: the OpenAPI output of the full route set, and which client generators accept it (aide writes 3.1).
