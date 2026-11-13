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

## Further settled decisions

- Public TurnSource uses core types; ActorSystem supplies the reference adapter. No HTTP types or Backend exposure.
- Private executor/resolver seams reuse handlers and guard/phase behavior; server TTL drives per-turn timing.
- Manual and background claims reserve the same capacity; stop also drains manual tasks on an unstarted Worker.
- Completion retains the current report and one fixed budget, including retryable failures receiving a final fallback.
- Source responses determine actual observer outcomes; unknown completion never authorizes handler re-execution/release.

## Capabilities

### New Capabilities

- `remote-workers`: Rust Worker over a public turn source, with shared execution, deadlines and lifecycle.

## Out of scope

HTTP implementation, another language's SDK, CLI integration, long-polling, and worker presence in GET /nodes.

## Order

After `complete-leased-turns`, before `remote-turns-over-http`.

## Impact

Core worker/runtime/transport and resolver seams, virtual-time behavior and fault tests, core guide, architecture and ADR 0039.
The direct-store runtime keeps its behavior; this change must prove that before the transport ships over HTTP.
