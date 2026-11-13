# Proposal

## Why

An App that hosts Rust actors should be deployable as an HTTP worker without custom bootstrap code or database
credentials. The preceding library worker provides that behavior; the command line needs to compose it.

## What Changes

- Allow the worker command to select an HTTP server and use the remote Worker instead of opening a Store.
- Reuse the application's actor selection, authentication/client conventions and shutdown handling.
- Document deployment and regenerate the command-line reference.

## To decide

- Flags, environment variables, context/token settings and conflicts between server and store configuration.
- Which NodeConfig settings translate to WorkerConfig, and diagnostics for unsupported settings.
- A small example and targeted end-to-end test proving no database is opened by an HTTP worker.
- Whether a plain headless cicada serve is wanted: if so, propose it separately; hosts can already mount an empty
  ActorSystem's HTTP router, so it is not required for this change.

## Out of scope

Changes to the wire protocol, a headless-server CLI change, other-language SDKs and worker registration in GET /nodes.

## Order

After `remote-turns-over-http`. No following change.

## Impact

App settings/command composition, CLI generation/tests, app guide and deployment example. Use the predecessor's
landed APIs rather than pinning a speculative signature here.
