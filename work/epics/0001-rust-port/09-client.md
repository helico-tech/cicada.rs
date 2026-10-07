---
status: in-progress
created: 2026-10-07
---

# 09. Typed HTTP client on reqwest

## Goal

Typed HTTP client on reqwest.

## Acceptance criteria

- Feature parity with `ActorClient.kt`; its tests ported against an in-process server.

## Plan

- Port `ActorClient.kt` to `cicada-client` on reqwest: typed methods over `cicada::api`, absent optional request
  fields left out, expected alternatives as values (`None`, `bool`, `CancelOutcome`), every other error response as
  `ClientError::Api(Problem)`.
- Sends carry a client-generated id and are retried with it on transport errors and `503`, with a doubling delay.
- Tests: port `ActorClientTest` and `EmbeddedServerTest` against a real axum server on `127.0.0.1:0`; Ktor client
  plugins become axum middleware on the test server (lost acknowledgement, 503, body recording, auth).

## Progress log

- (none yet)

## Follow-ups

- (none yet)
