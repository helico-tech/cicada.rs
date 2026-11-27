# Proposal

## Why

When something fails, operators need the failed message: its error, its attempts, its payload and what caused it.
Mockup: https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH, page Inspector, "Outcomes and a failed message".

## What Changes

- Outcomes in the sidebar shows finished messages (`/outcomes`) with a Failed / Cancelled / Done / All switch:
  message type, actor, attempt, finished time and error. The sidebar shows the failed count (`/stats`, from
  `stats-failed-count`).
- Selecting one fills the pane with the message (`/messages/{id}`): the error, attempts and retry policy, the
  envelope (to, from, correlation id, finished), the payload as highlighted JSON, and the chain that caused it
  (`causedBy`, one step at a time) and the messages its turn sent (`/caused`).
- Show Actor jumps to the actor in Actors. Back and forward follow these jumps.
- Running Turns and Nodes get their tables, with the running turns of a node in the pane (`/nodes/{node}`).
- Read only, as in `inspector-actors`.

## Out of scope

- Retry and cancel (`inspector-actions`), live updates (`inspector-live`).

## Order

After `inspector-types-watches`, before `inspector-live`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector`. Tests of the mapping against an in-process server; a manual check list. Docs handoff: the
app guide.
