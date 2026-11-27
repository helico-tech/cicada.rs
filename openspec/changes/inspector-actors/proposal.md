# Proposal

## Why

The inspector window from `inspector-window` shows only the Overview. Operators most often need one actor: its
state, its mailbox, its running turn and its children. Mockup: https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH,
page Inspector, "Actors".

## What Changes

- Actors in the sidebar shows a table of actors (`/actors/states`): address, type, version, created and status
  (running from `/turns`, paused from the actor). The search field filters by address prefix. The table pages as it
  scrolls.
- Selecting an actor fills the inspector pane: its state as highlighted JSON, its running turn (message, attempt,
  time, node), its parent, created, pinned node and paused flag, and tabs for its mailbox (`/mailbox`) and its watches
  (`/watches`).
- Folding a parent open to its children moved to the next change, `inspector-children`, and Messages, Types and
  Watches to `inspector-tables` (design, 2026-11-27: all four did not fit
  five tasks).
- A toolbar button shows and hides the pane.
- Everything is read only. The buttons for actions are absent until `inspector-actions`.

## Out of scope

- Outcomes and message details (`inspector-outcomes`), live updates (`inspector-live`), actions (`inspector-actions`).
- Editing state.

## Order

After `inspector-in-tray`, before `inspector-children`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (tables, pane, outline). Tests of the mapping from API pages to rows against an in-process
server; a manual check list for the window. Docs handoff: the app guide.
