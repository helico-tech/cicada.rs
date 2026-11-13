# Proposal

## Why

A Rust worker outside the database network should require only a Cicada HTTP URL and host-provided credentials.
The preceding core worker and lease APIs need an HTTP transport usable through a load balancer.

## What Changes

- Expose claim, renew, release and complete as language-neutral JSON operations on the full HTTP router.
- Implement the core worker transport for ActorClient and publish requests/results in OpenAPI.
- Exercise an unchanged Rust actor through HTTP, leasing on one server and finishing through another.
- Document lost responses, body limits, timeouts, trusted workers and ordinary host authentication.

## To decide

- Final routes and exact status/problem mappings, including lease loss, invalid reports and rejected effects.
- Preserve raw encoded actor paths, duration precision, required envelope fields and source error categories.
- Bound request time relative to the lease, avoid hidden claim retries, and retry completion without rerunning code.
- HTTP and restart tests with independent servers on a shared store, plus a real Worker/ActorClient example.

## Out of scope

Sticky sessions, a durable receipt service, worker-specific credentials or quotas, long-polling/push dispatch,
another language's SDK, CLI configuration, and a new remote ActorSystem facade for every management operation.

## Order

After `remote-worker-runtime`, before `app-worker-over-http`.

## Impact

Axum lease handlers/routes, client transport, OpenAPI, HTTP/fault integration tests, HTTP and operations guides,
and a wire-format ADR. This is the first end-to-end remote-worker release for library hosts.
