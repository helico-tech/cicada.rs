# Proposal

## Why

Until now the inspector refreshes by hand. Operators watching a running system want the screens to follow it: the
mockup's Live pill (https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH, page Inspector).

## What Changes

- The inspector listens to `GET /changes` (Server-Sent Events, ADR 0035) for the prefix on screen. A hint reloads the
  rows and the pane it touches, at most a few times a second.
- `/changes` is best effort, so the screen also reloads on a slow timer, and after the stream drops and comes back.
- The toolbar shows Live, Reconnecting or Paused. Clicking it pauses and resumes the updates.
- The selection, the scroll position and the folded rows stay as they are across reloads.

Split by its design review (2026-11-27): the stream reader is `client-change-stream` and the bounded refresh that keeps
the view is `inspector-soft-refresh`; this change keeps the scheduling and the pill, and its design is rewritten on
them.

## Out of scope

- A change to `/changes` itself.
- Live updates in the tray's menu, which already follows its own worker.

## Order

After `inspector-soft-refresh`, before `inspector-actions`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the stream's lifecycle and scheduling on `client-change-stream`, reloads through
`inspector-soft-refresh`, the Live pill). Tests on virtual time with an in-process
server sending hints. Docs handoff: the app guide.
