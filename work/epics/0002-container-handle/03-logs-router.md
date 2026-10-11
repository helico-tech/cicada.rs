---
status: done
created: 2026-10-11
---

# 03. Logs router

## Goal

`container_logs(engine)`: an axum `Router` with `GET /containers/{name}/logs` that streams a handle's container output from Docker.

## Acceptance criteria

- Query: `tail` (number or `all`, default 100), `follow` (default false), `since` (RFC 3339), `timestamps`.
- `text/plain` body, streamed; works with `curl -N`.
- Only containers with the handle's `cicada.uri` label; others and unknown names are `404` problem+json; a bad query is `400`.
- Tests with the fake engine.

## Plan

`container_logs(&ContainerActor) -> Router`: validate the query, resolve the container name, inspect and require the
handle's `cicada.uri` label, then `Body::from_stream` of the engine's log stream. Problems are rendered with
`cicada::api::Problem`; a Docker failure is `502` with `urn:cicada:problem:docker-unavailable`.

## Progress log

- 2026-10-11: Built; 5 tests with `tower::ServiceExt::oneshot` on the fake engine (default tail, `tail`, 404 for
  unknown, foreign and invalid names, 400 for bad queries, 502 when Docker is down).
- Takes the `ContainerActor` instead of the engine, so it uses the same prefix.

## Follow-ups
