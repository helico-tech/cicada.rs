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

## Out of scope

- A change to `/changes` itself.
- Live updates in the tray's menu, which already follows its own worker.

## Order

After `inspector-nodes`, before `inspector-actions`.

## Impact

Crates: cicada-client, cicada-inspector.

`crates/cicada-client` (a reader for `/changes`, which it lacks), `crates/cicada-inspector` (the stream's lifecycle,
the throttle, keeping the selection). Tests on virtual time with an in-process
server sending hints. Docs handoff: the app guide.
