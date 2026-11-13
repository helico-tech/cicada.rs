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

## To decide

- Event names and JSON data, keep-alive interval, and how OpenAPI describes the stream.
- HTTP shutdown/disconnect and store-close behaviour, subscription resource limits and Last-Event-ID handling
  (there is no replay history).
- Whether a typed cicada-client streaming method is needed in this small change or browser EventSource is sufficient.

## Capabilities

### New Capabilities

- `change-feed-http`: expose the core actor change hints over Server-Sent Events.

### Modified Capabilities

- `management-api`: the read/full routers and OpenAPI describe the new streaming read.

## Out of scope

State or version in events, durable replay/history, exactly-once delivery, SQLite coverage beyond the core feed's
shared Store, a snapshot across pages, state-field filters, CLI/TUI integration, actor contract and SQL schema changes.

## Order

After `actor-change-feed`. No following change.

## Impact

`cicada-axum` route/handler/tests and OpenAPI, HTTP guide and tested browser guidance, architecture and an ADR for
SSE wire conventions, changelog/version; cicada-client only if the design chooses to include it. The core feed is
the predecessor and supplies all event semantics. Host-specific code-factory adoption is outside this proposal.
