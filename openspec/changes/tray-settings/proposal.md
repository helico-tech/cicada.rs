# Proposal

## Why

The tray connects to the current context of the config file (`tray-menu-bar`). Users also need to configure it:
pick or add a context, set the server and token, choose the types this worker runs, and set how many turns run at
once. The mockup's Settings window shows it (https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH). It is a native
window, like the menu (owner's choice, 2026-11-26).

## What Changes

- Settings… opens an `NSWindow` built with `objc2-app-kit`, in the grouped style of System Settings:
  - Connection: a context pop-up with "Add…", the server, and the token (a secure text field).
  - Worker: a switch per registered type (`--types`), and the turns at once (`--concurrency`).
  - "Revert" and "Apply and Reconnect". Applying disconnects and connects again.
- Contexts are the ones `cicada config` keeps, read and written by the same code, so the command line and the tray
  share them. `cicada-app` exposes that code to the tray crate.
- The types and the concurrency are kept per context in the tray's own small file next to the config file.
- The window opens once; Settings… brings it to the front. It closes with ⌘W and Esc.

## Out of scope

- The macOS Keychain. The token is kept where `cicada config` keeps it today.
- Start at login.
- Editing the node settings other than the concurrency.

## Order

After `tray-menu-bar`, before `stats-failed-count`.

## Impact

Crates: cicada-tray, cicada-app.

`crates/cicada-tray` (the window in the `appkit` module; reading and writing the settings in a plain module),
`crates/cicada-app` (config code made reachable by the tray). Tests for reading and writing the files, and for
turning the form's values into worker arguments; the window gets a manual check list. Docs handoff: the app guide.
