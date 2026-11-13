# 0042. App workers over HTTP

Date: 2026-11-13

## Status

Accepted for `app-worker-over-http`. Amends ADR 0027's refusal of operator flags for worker only. The owner requested
unchanged Rust actors leasing through any HTTP server sharing the database and delegated technical design choices.

## Context

Worker and ActorClient's TurnSource support database-free actor executors. App already owns actor selection, observers,
deployment settings and shutdown. Its worker command should compose those pieces without changing direct-store workers
or treating a token passed to serve as server authentication.

## Decision

- Let worker accept --server, --context and --token; continue refusing output formatting. serve still refuses all
  operator flags. Neither command becomes available in a binary without actor types.
- Explicit --store conflicts with --server/--context. Otherwise explicit mode flags win. Without flags, CICADA_STORE
  keeps precedence over HTTP target variables, preserving existing deployments with ambient operator configuration.
  current-context and tokens alone do not select HTTP mode.
- Only HTTP mode loads operator config and uses its target/token/header resolution. A context may still supply its
  token when an explicit server overrides its URL, as for operators. Direct-store workers ignore the config file.
- Map applicable node settings to WorkerConfig, preserving flag/environment/host/default precedence. Refuse explicit
  server-owned settings in HTTP mode and ignore their ambient/host values. Do not parse/open/check a store in that path.
- Share type/observer hosting between private builder variants. Reject on_start hooks in HTTP mode because their
  ActorSystem cannot be supplied; do not silently skip bootstrap work or introduce a new hook API in this change.
- Build the worker's HTTP client with redirects disabled, the existing auth/header conventions and no added retries.
  Use the existing turn timeout and a 30s total client timeout. Redact credential fields in new diagnostics.
- Probe type listing while unavailable before start, at 1s intervals with no new probe after 60s. The last request
  may finish later. Stop interrupts this wait; permanent errors and subsequent registration/start errors fail promptly.
- Run until a stop signal or automatic Worker failure. Drain through Worker::stop, preserve a recorded failure in a
  signal race, and never close the remote source. Cancellation and completion policy remain those of Worker.

## Consequences

- One App definition can run actors beside a database or behind HTTP, and HTTP workers compile without a store feature.
  Existing store-worker/server behavior, including on_start and host HTTP layers, remains available.
- An App using on_start must separate bootstrap from HTTP-worker execution before adopting this mode. Refusing it is
  explicit and can be extended by a future hook design without guessing how to translate ActorSystem operations.
- A readiness probe cannot guarantee registration succeeds next. Temporary failures during registration fail startup;
  no string-based retry of StartError is added. Authentication/wrong-server failures do not wait a minute to fail.
- Graceful stop remains cooperative; code that ignores cancellation may outlive the wait. App adds no forced release,
  worker presence, HTTP readiness listener, database migration or wire-format change.
