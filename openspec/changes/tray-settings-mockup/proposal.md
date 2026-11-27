# Proposal

## Why

The owner found on 2026-11-27 that the tray and the inspector do not look like the mockup ([MOCK-FIX]). The Settings
window was built from `tray-settings`' words, not from its board, and nobody has compared it with the board yet.

## What Changes

Match the board in `mockup/Settings.dc.html`: a "Connection" group (Context pop-up with Add…, Server, Token), a "Worker"
group (a switch per type, "Turns at once" with "The worker's concurrency"), the footer note "Contexts are shared with
`cicada config`" with Revert and Apply and Reconnect (default). The design starts from the owner's screenshot of the
window today, compares every element and lists the departures for the owner to agree.

## Order

After `tray-menu-mockup`. Before `inspector-overview-mockup`.

## Impact

Crates: cicada-tray.

`crates/cicada-tray` (the Settings window's AppKit view and its portable form). A manual check list next to the board,
light and dark. Docs handoff: the app guide's Settings text, if what users see changes.
