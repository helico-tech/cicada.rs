# Proposal

## Why

With the inspector read only, operators still switch to `cicada` to act on what they find. The owner wants actions
after the read-only screens (2026-11-26).

## What Changes

- Actor pane: Pause and Resume, Cancel Turn, Purge Mailbox… and Delete… (with its descendants).
- Send Message… moved to `inspector-send-message`, and the message pane's and Outcomes' actions to
  `inspector-message-actions` (design, 2026-11-27: all of it did not fit five tasks).
- Cancel Turn, Purge and Delete ask first in a sheet that names what goes. An absent actor shows local text;
  other refusals show as alerts with the server's message.
- The actions use the same routes as `cicada`, through `cicada-client`, so the server's middleware and tokens apply.

## Out of scope

- Creating actors and registering or deleting types.
- Undo.

## Order

After `inspector-live`, before `inspector-send-message`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (sheets, alerts, actions). Tests of each action against an in-process server; a manual check
list. Docs handoff: the app guide.
