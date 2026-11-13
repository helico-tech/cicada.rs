# Proposal

## Why

A remote worker has an actor's Rust implementation, but its HTTP server need not. The worker must register the
actor's contract before that server can accept creates and messages. Today only starting a hosting node registers it.

## What Changes

- Add a public core operation to register a Contract without hosting or starting its handler.
- Add PUT /types/{name} with state and message schemas, and a typed client method.
- Compute the hash on the server and preserve the existing atomic, first-registration-wins contract comparison.
- Specify malformed requests, invalid contracts, conflicts and the relationship to ordinary node initialization.

## Settled direction

The owner chose full actor turns over HTTP, existing Rust handlers first, a language-neutral protocol, and lease
operations through any server sharing the database. The series is: registration, core leases, completion reports,
Rust worker runtime, HTTP leases, then CLI integration. Registration is independently useful and changes no execution.

## Capabilities

### Modified Capabilities

- `type-registry`: register a contract through the public core API and HTTP, without a hosted implementation.

## Out of scope

Leasing or running turns, worker membership, contract updates or migrations, replacing registered contracts, a new
authentication system, schema compilation at registration, CLI commands, and SQL schema changes.

## Order

After `actor-change-sse`, before `lease-turns-in-core`.

## Impact

Core registration/result and shared request types; axum handler/router/OpenAPI; typed client; registry, HTTP and
client tests; ADR 0036, architecture and the core/HTTP guides. Backend registration semantics remain unchanged.
