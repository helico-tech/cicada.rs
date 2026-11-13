# Proposal

## Why

A Rust worker outside the database network should require only a Cicada HTTP URL and host-provided credentials.
The preceding core worker and lease APIs need an HTTP transport usable through a load balancer.

## What Changes

- Expose claim, renew, release and complete as language-neutral JSON operations on the full HTTP router.
- Implement the core worker transport for ActorClient and publish requests/results in OpenAPI.
- Exercise an unchanged Rust actor through HTTP, leasing on one server and finishing through another.
- Document lost responses, body limits, timeouts, trusted workers and ordinary host authentication.

## Settled decisions

- Four full-router POST routes under /leases carry canonical addresses in JSON and core alternatives in tagged results.
- Requests reject unknown envelope fields and duplicate keys; execution envelopes require all fields and valid policies.
- ActorClient implements TurnSource, with bounded turn calls, explicit error mapping and no application-level retries.
- Existing body limits and host authentication apply; failed or oversized completion never permits hidden fallback/release.

## Capabilities

### New Capabilities

- `remote-turn-http`: language-neutral turn leasing/completion and the Rust HTTP TurnSource adapter.

## Out of scope

Sticky sessions, a durable receipt service, worker-specific credentials or quotas, long-polling/push dispatch,
another language's SDK, CLI configuration, and a new remote ActorSystem facade for every management operation.

## Order

After `remote-worker-runtime`, before `app-worker-over-http`.

## Impact

Axum lease handlers/routes, client transport, OpenAPI, HTTP/fault integration tests, HTTP and operations guides,
and a wire-format ADR. This is the first end-to-end remote-worker release for library hosts.
