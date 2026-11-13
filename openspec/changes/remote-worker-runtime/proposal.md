# Proposal

## Why

The owner wants existing Rust Actor implementations to run remotely without rewriting their handlers. A worker
needs the same Turn behavior and cancellation discipline as a local node, using a transport instead of a database.

## What Changes

- Introduce Worker and its transport interface, configured with hosted actors, observers and runtime limits.
- Register contracts at startup, poll for leases, and run handlers through shared executor code.
- Keep registry-dependent raw sends/creates, time, timeout, panic/decode handling, renewal and buffered effects.
- Test against an in-process ActorSystem transport and deterministic fault-injecting transports.

## Settled decisions

- The worker executes a complete actor turn; it is not a job consumer reporting to another actor.
- Existing Rust Actor/Turn handlers come first, with a language-neutral protocol for later clients.
- Renew and complete may reach any server sharing the database; no sticky session or server-local lease owner.

## To decide

- Public Worker/WorkerConfig and transport APIs; reuse executor and resolver without leaking private Backend types.
- Conservative monotonic deadlines from request start, including transport delay, retries and late responses.
- Cancellation versus handler completion, MAX_TURN, shutdown, G6, and renewal during completion.
- Startup hash checks, dynamic registry reads, observer outcomes after uncertain commits, and bounded concurrency.
- Report server policy rather than duplicate retry/claim-limit policy in the worker.

## Out of scope

HTTP implementation, another language's SDK, CLI integration, long-polling, and worker presence in GET /nodes.

## Order

After `complete-leased-turns`, before `remote-turns-over-http`.

## Impact

Core worker/runtime/transport and resolver seams, virtual-time behavior and fault tests, core guide and architecture.
The direct-store runtime keeps its behavior; this change must prove that before the transport ships over HTTP.
