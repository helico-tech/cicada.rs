# Proposal

## Why

The inspector window from `inspector-window` shows only the Overview. Operators most often need one actor: its
state, its mailbox, its running turn and its children. Mockup: https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH,
page Inspector, "Actors".

## What Changes

- Actors in the sidebar shows a table of actors (`/actors/states`): address, type, version, created and status
  (running from `/turns`, paused from the actor). A parent folds open to show its children (`/children`). The
  search field filters by address prefix. The table pages as it scrolls.
- Selecting an actor fills the inspector pane: its state as highlighted JSON, its running turn (message, attempt,
  time, node), its parent, created, pinned node and paused flag, and tabs for its mailbox (`/mailbox`) and its watches
  (`/watches`).
- Messages (the pending messages under a prefix, `/messages`), Types (`/types` with their schemas) and Watches get
  their tables with the same pane.
- A toolbar button shows and hides the pane.
- Everything is read only. The buttons for actions are absent until `inspector-actions`.

## Out of scope

- Outcomes and message details (`inspector-outcomes`), live updates (`inspector-live`), actions (`inspector-actions`).
- Editing state.

## Order

After `inspector-window`, before `inspector-outcomes`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (tables, pane, outline). Tests of the mapping from API pages to rows against an in-process
server; a manual check list for the window. Docs handoff: the app guide.
