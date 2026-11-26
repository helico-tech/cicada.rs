# Proposal

## Why

`tray-worker-control` gives a model with no GUI. Users need the menu itself: an icon in the macOS menu bar, and a
menu that shows the state, the running turns, the contexts and the registered types, and connects and disconnects.
The owner wants it to look and feel native, as in the mockup (https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH),
and chose real AppKit over a web view (2026-11-26): only AppKit gives native keyboard use, vibrancy, menu timing,
dark mode and VoiceOver.

## What Changes

- `Tray::run()` owns the main thread, as AppKit requires, and runs tokio on a background thread. State changes reach
  the main thread through a channel; menu actions go back to the model the same way.
- A status item (`NSStatusItem`) with a template icon that follows the menu bar's light or dark look. It is dimmed
  when offline.
- A real `NSMenu` built with `objc2-app-kit`, using the menu features of macOS 14:
  - A header with the app name, the state ("Connected to prod") and an `NSSwitch` that connects and disconnects.
    This is the only custom view.
  - Section headers for Running and Context.
  - Running turns as items with a subtitle (message type and attempt) and the time since the claim.
  - Contexts as items; the current one is marked, and picking another reconnects.
  - Submenus for Registered Types (a checkmark for the types this worker runs) and Recent Turns (finished turns, a
    badge with the number of failures).
  - Settings… and Quit, with their key equivalents.
- The menu is rebuilt from the state while it is open, so the running turns and the times stay current.
- While disconnecting, the header shows the time left in the grace period and the menu offers Stop Now.
- Icons are SF Symbols, through `NSImage`'s system symbol names.
- The crate builds on macOS 14 or later (`cfg(target_os = "macos")`, a deployment target of 14.0). Elsewhere it is
  empty.

## Out of scope

- The settings window (`tray-settings`). This change connects to the current context of the config file.
- Building a `.app` bundle, signing and notarising it. The guide names the steps (including `LSUIElement`, so the app
  has no Dock icon); the user's build does them.
- Older macOS versions, Windows and Linux.

From the design review of `tray-worker-control` (2026-11-26): the tray's model is portable (not empty outside macOS);
only the menu is macOS-only. "Stop Now" (cancel running turns at once) and a checkmark for the types this worker runs
need model support this change adds or names.

## Order

After `app-node-id-after-types`, before `tray-settings`.

## Impact

`crates/cicada-tray` (an `appkit` module with the status item, the menu and the header view). New dependencies
`objc2`, `objc2-foundation` and `objc2-app-kit`, macOS only; `cargo deny` must pass. The `unsafe` calls into AppKit
stay in that module, each with its reason. The gate can't drive the GUI: the mapping from state to menu items is a
plain function with unit tests, and the menu gets a manual check list in the tasks. Docs handoff: the app guide and a
worked example in `examples/`.
