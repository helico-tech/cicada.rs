# Proposal

## Why

An App that hosts Rust actors should be deployable as an HTTP worker without custom bootstrap code or database
credentials. The preceding library worker provides that behavior; the command line needs to compose it.

## What Changes

- Allow the worker command to select an HTTP server and use the remote Worker instead of opening a Store.
- Reuse the application's actor selection, authentication/client conventions and shutdown handling.
- Document deployment and regenerate the command-line reference.

## Settled decisions

- Explicit --server/--context select HTTP; explicit --store selects the store; conflicting mode flags fail.
- Without mode flags, CICADA_STORE keeps precedence over ambient HTTP target variables; current-context alone does
  not switch a worker's mode. HTTP target/auth resolution reuses operator conventions.
- Map only applicable host/flag/env settings into WorkerConfig. Refuse explicit server-owned settings and ignore
  their ambient values; remote execution never opens a store or requires a store feature.
- Share actor/observer hosting, disable redirects on the worker client and preserve Worker stop/error semantics.
- Reject ActorSystem-only on_start hooks in HTTP mode before network access. No headless-server or new hook API.

## Capabilities

### Modified Capabilities

- `app`: HTTP worker selection, configuration, lifecycle and documentation alongside the existing store worker.

## Out of scope

Changes to the wire protocol, a headless-server CLI change, other-language SDKs and worker registration in GET /nodes.

## Order

After `remote-turns-over-http`. Before `deep-failure-notices`.

## Impact

App settings/command composition, CLI generation/tests, app guide and deployment example. Use the predecessor's
landed APIs rather than pinning a speculative signature here.
