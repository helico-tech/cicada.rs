# Proposal

## Why

The core change feed tells an in-process host when to reread an actor or relist a prefix. A browser page needs the
same hints over HTTP, so the code-factory showcase and other dashboards can refresh without polling all actors.

## What Changes

- Add GET /changes?prefix= as Server-Sent Events on the full and read-only routers, under the host's existing layers.
- Map the core Changed and Resync items to SSE events, retaining the feed's initial resync and best-effort scope.
  This is transport only: it adds no cross-process coverage, durability, ordering or loss-detection guarantee.
- Define event names/data, keep-alives, cancellation/shutdown and reconnect guidance; describe text/event-stream
  in OpenAPI and show a browser resync/read loop using the actor-state listing.

## Settled decisions

- `changed` carries JSON {"address":"<canonical>"}; `resync` carries {}. Emit no id/retry fields and ignore
  Last-Event-ID. Every reconnection gets the core's fresh readiness/resync boundary.
- Return 200 immediately, including while readiness is pending, with 15-second keep-alive comments. Store close
  ends the body; worker stop alone does not. Keep the app's existing bounded drain and host-owned resource limits.
- Publish text/event-stream in OpenAPI using a private output marker. No cicada-client streaming method or new
  public event DTO: browser EventSource and in-process changes meet this scope. ADR 0035 records the wire convention.

## Capabilities

### New Capabilities

- `change-feed-http`: expose the core actor change hints over Server-Sent Events.

### Modified Capabilities

- `store-backends`: HTTP change streams end normally after close, the streaming exception to ordinary 503 reads.

## Out of scope

State or version in events, durable replay/history, exactly-once delivery, SQLite coverage beyond the core feed's
shared Store, a snapshot across pages, state-field filters, CLI/TUI integration, actor contract and SQL schema changes.

## Order

After `actor-change-feed`. No following change.

## Impact

`cicada-axum` route/handler/tests and OpenAPI, an app shutdown regression test, HTTP guide and protocol-tested browser
guidance, architecture, ADR 0035 and changelog/version. No new client method or dependency. The core feed is
the predecessor and supplies all event semantics. Host-specific code-factory adoption is outside this proposal.
