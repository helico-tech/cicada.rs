# 0035. Actor change hints over Server-Sent Events

Date: 2026-11-13

## Status

Accepted for `actor-change-sse`. The owner delegated design decisions to an Opus 5.5 adviser.

## Context

ADR 0034 gives in-process hosts bounded change hints and resync recovery. Browser pages need the same hints over HTTP,
with no additional state/version delivery or replay promise. The existing read/full routers and application shutdown
already supply the required authentication boundary and bounded HTTP drain.

## Decision

- Serve GET /changes?prefix= on both routers, operationId streamActorChanges, using the usual literal query-prefix
  rules. Use text/event-stream with Cache-Control: no-cache and the proxy hint X-Accel-Buffering: no.
- Map Changed to event changed with JSON {"address":"<canonical>"}, and Resync to event resync with JSON {}.
  No id or retry fields and no replay: ignore Last-Event-ID, and each new subscription begins with the core's resync.
- Return 200 immediately without a store preflight. Keep-alive comments say keep-alive every 15 seconds of pending
  stream time. They may precede readiness; the first application event waits for the core source to become Up.
- Store close ends the body without a final event; subscribing after close returns an empty ended 200 stream.
  This is a streaming exception to the ordinary store-unavailable HTTP rule. Worker stop alone leaves streams open.
- Preserve the application's existing node-stop, five-second HTTP drain, forced body cutoff and store-close sequence.
  Standalone hosts own their shutdown/connection limits. No additional server event queue, quota or configuration.
- Describe the 200 representation as an OpenAPI 3.1 string with text/event-stream media type, examples and event
  descriptions, plus ordinary pre-stream problem errors. Use private framing/documentation types only.

## Consequences

- Browser EventSource consumers refresh an address after changed and relist after resync. They need bounded pending
  work and serialized reads, must preserve canonical address encoding, and cannot rely on events as a durable history.
- Native EventSource has no option for custom authorization headers; hosts choose session/cookie auth or a different
  client and configure CORS as appropriate. Proxy buffering/timeouts may still need explicit host configuration.
- Open dashboards may consume the whole app HTTP drain. Body errors/cutoffs are transport failures, not new SSE events.
- No new cicada-client streaming API, actor contract, database schema or dependency. Remote Rust consumers supply
  their own SSE reader; in-process hosts keep using ActorSystem::changes.
- ADR 0034's coverage, omissions and reconciliation limits remain unchanged. The architecture and guides document
  the endpoint, closed-store exception and consumer recovery protocol when implemented.

The [HTML Standard's EventSource protocol](https://html.spec.whatwg.org/multipage/server-sent-events.html) defines
named events, nonempty data dispatch, comments and reconnection; the JSON resync payload ensures it is dispatched.
