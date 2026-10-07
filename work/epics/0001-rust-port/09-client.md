---
status: done
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

- 2026-10-07: `ActorClient` (`new`, `with_client`, `send_attempts`, `retry_delay`) with the operations of
  `ActorSystem` by the same names (`create_actor`/`create`/`create_stateless`, `get_actor`/`get`/`state`,
  `delete_actor`/`delete`, `list_actors`, `send_message`/`send`/`send_with` taking the core `SendOptions`, `pending`,
  `list_pending`, `status`, `chain`, `cancel`, `failures`, `types`, `type_of`, `health`). Errors: `ClientError::Api`
  (server problem, or `about:blank` made up from the status), `Http`, `Body`, `Invalid`; `Display` reads like the
  Kotlin `ActorApiException` message.
- 2026-10-07: Sends retry on any reqwest error except builder errors, and on 503, with the same id and a doubling
  delay.
- 2026-10-07: The `url` crate removes `%2E`/`%2E%2E` segments, so the client refuses addresses with `.` or `..`
  segments instead of misaddressing them ([0300](../../issues/0300-client-dot-segments.md)).
- 2026-10-07: Tests: `client.rs` (ActorClientTest, 19) and `embedded.rs` (EmbeddedServerTest) against a real axum
  server on `127.0.0.1:0`. Ktor client plugins became server middleware: a broken response body after the API
  accepted the send (lost acknowledgement), a 503 for the first POST, request body recording, basic auth. `health
  follows the store` checks the 503 → false mapping by stopping the node instead of a flaky store. Checked by hand that
  a 2 and 8 MiB body reaches the client as the server's 413, not as a transport error.

## Follow-ups

- Run `embedded.rs` on `SqliteStore` once story 05 lands.
- [0300](../../issues/0300-client-dot-segments.md): addresses with `.` or `..` segments.
