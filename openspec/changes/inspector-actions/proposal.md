# Proposal

## Why

With the inspector read only, operators still switch to `cicada` to act on what they find. The owner wants actions
after the read-only screens (2026-11-26).

## What Changes

- Actor pane: Pause and Resume, Send Message… (a sheet with the message type from the type's schema and a JSON
  payload checked against it), Cancel Turn, Purge Mailbox… and Delete… (with its descendants).
- Message pane: Retry, Cancel and Deliver Now, by the state of the message.
- Outcomes: retry or cancel the selected rows at once (`/messages/bulk-retry`, `/messages/bulk-cancel`).
- Destructive actions (Purge, Delete, bulk Cancel) ask first in a sheet that names what goes. The API's refusals
  (409, 404, a schema mismatch) show as alerts with the server's message.
- The actions use the same routes as `cicada`, through `cicada-client`, so the server's middleware and tokens apply.

## Out of scope

- Creating actors and registering or deleting types.
- Undo.

## Order

After `inspector-live`. No following change.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (sheets, alerts, actions). Tests of each action against an in-process server; a manual check
list. Docs handoff: the app guide.
