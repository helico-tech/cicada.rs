# Proposal

## Why

Operators want to send a message to an actor from the inspector, as `cicada send` does (owner's proposal for
`inspector-actions`, split off by its design, 2026-11-27).

## What Changes

- The actor pane's Send Message… opens a sheet: the message type from the actor's type's contract (`/types`), a JSON
  payload checked against that message's schema before sending, and Send through `send_message`.
- The server's refusal (a schema mismatch, an unknown type, 404) shows in the sheet with its message.

To decide in the design: how the payload is checked locally (the schema subset the inspector supports) and the editor.

## Order

After `inspector-actions`, before `inspector-message-actions`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the sheet and its model). Tests against an in-process server; a manual check list. Docs
handoff: the app guide.
