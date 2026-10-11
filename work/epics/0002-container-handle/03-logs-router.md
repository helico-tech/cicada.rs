---
status: todo
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

## Progress log

## Follow-ups
