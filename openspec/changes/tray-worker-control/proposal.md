# Proposal

## Why

Library users want a default way to run their actors from a Mac menu bar, the way `App` is the default binary
(owner's yes, 2026-11-26: a new crate, macOS only, showing the turns that run on this worker). The menu needs a model
it can show and drive: connect, disconnect, the state of the connection, the types that are registered, and the turns
that run. That model needs no GUI, so it can be built and tested first. Mockup:
https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH

## What Changes

- A new crate `cicada-tray`, without a GUI in this change. `Tray::new(|| App::new().host(MyActor))` takes a function
  that builds the `App`, because `App::run_with` uses the `App` up and each connect needs a fresh one.
- `connect(context)` starts `worker --context <name>` in the process through `run_with`, with a stop signal.
  `disconnect()` fires it, so the node stops claims and drains within its grace period, as on SIGTERM. Connecting
  again builds a new `App`.
- A state the menu reads and watches: `Offline`, `Connecting`, `Online`, `Stopping` (with the grace period's end) and
  `Failed` (with the error). A `TurnObserver` added by the tray tracks the running turns (actor, message type, attempt,
  claim time) and keeps a short list of finished turns with their outcome.
- `cicada-app` gains `App::type_names()`, so the menu can list the registered types.
- An ADR for the new crate and its place next to `cicada-app`.

## Out of scope

- The menu bar icon and menu (`tray-menu-bar`), settings and contexts (`tray-settings`).
- A worker on the store, and background tasks. A binary with `App::task` or `App::on_start` can't run over HTTP, so
  the tray refuses such an `App` with a clear error.
- Windows and Linux.

## Order

After `change-loop-two-builders`, before `placement-settings`.

## Impact

New crate `crates/cicada-tray`; `crates/cicada-app` (`type_names`); a new ADR and its row; the workspace manifest.
Tests on virtual time against an in-process server. Docs handoff: a short section in the app guide.
