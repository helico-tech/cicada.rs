# Proposal

## Why

Operators want to look inside a running system from a Mac with a native app, not only through `cicada` and its
terminal UI. The owner chose a native inspector over the HTTP API, opened from the tray or on its own (2026-11-26).
Mockup: https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH, page Inspector. ADR 0011 says the repository holds no
console; the owner agreed to supersede it, now that the design is fixed.

## What Changes

- A new ADR that supersedes ADR 0011: the repository holds a native macOS inspector, built in Rust on the HTTP API
  through `cicada-client`. Still no browser console and no container handles.
- A new crate `cicada-inspector`, macOS 14 or later, with AppKit through `objc2-app-kit`, as in the tray:
  - The window: a see-through sidebar (Overview; Browse: Actors, Messages, Running Turns, Outcomes, Nodes, Types,
    Watches), a toolbar with back and forward, the title, and a context pop-up shared with `cicada config`.
  - The Overview: tiles for actors, pending messages, oldest due and failed messages (`/stats`), the health
    (`/health`), the nodes (`/nodes`), the running turns (`/turns`) and the types with their actor counts (`/types`,
    `/stats` per prefix).
  - The other sidebar items show "Coming soon" until their change lands.
  - Refresh by hand (a toolbar button and ⌘R).
- `Inspector::run()` opens it on its own; a host makes a small binary or `.app` with it.
- `cicada-tray` gains a feature `inspector`. With it, the menu has "Open Inspector… ⌘I", which opens the window in the
  tray's process, on the tray's context.

## Out of scope

- Actors, outcomes, live updates and actions: the following changes.
- A `.app` bundle, signing and notarising.
- Windows and Linux.

## Order

After `stats-failed-count`, before `inspector-actors`.

## Impact

Crates: cicada-inspector, cicada-tray, cicada-app.

New crate `crates/cicada-inspector`; `crates/cicada-tray` (feature and menu item); the new ADR, its row, and
ADR 0011's status line; the workspace manifest. As in the tray: the mapping from API answers to what the window shows
is plain code with tests against an in-process server, and the window gets a manual check list. Docs handoff: the
app guide and the docs index.
