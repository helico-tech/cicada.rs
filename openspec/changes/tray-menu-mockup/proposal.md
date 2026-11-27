# Proposal

## Why

The owner ran the menu bar app on 2026-11-27: it "looks nothing like" the mockup ([MOCK-FIX]). `tray-menu-bar` chose
plain native menu items with only the header as a custom view, and named no departure from the boards. Seen in the
owner's screenshot (offline): context rows are a checkmark and a name; the header says "Offline"; an empty "Running"
header shows; the submenus show no counts; the order is Settings, Open Inspector, Quit, with no "Connect to" row.

## What Changes

Match the boards in `mockup/` (Main, Disconnected, Disconnecting, Running, Registered):

- Context rows with a circle icon, the name and the server's host on a second line; the current one filled.
- Running rows with a circle icon, the actor, "<type> · attempt N" and the elapsed time; "Running 3 of 4" (running of
  the worker's limit); no Running section while not connected.
- Not connected: "Not connected" under the name, the last connection failure as a row ("Couldn't reach <context>"
  with a fixed label and when, no error text), and "Connect to <context>" (⌘K).
- Disconnecting: the grace period as a bar with "Taking no new turns" and the seconds left, the "Finishing" turns, Stop
  Now with its explanation.
- Submenu rows with their counts ("3 of 4", "1 failed"); Registered Types as "Actor Types" with each type's state, a
  "Background Tasks" section and "Choose Types…"; Recent Turns rows with a done or failed icon, the line and the time,
  and "Kept until the app quits".
- The order Open Inspector…, Settings…, Quit <app>.

The design compares every element of each board with the menu today and lists any departure (for example what
`NSMenu` cannot draw) for the owner to agree.

## Order

After `gui-mockup-boards`. Before `tray-settings-mockup`.

## Impact

Crates: cicada-tray, cicada-app.

`crates/cicada-tray` (`src/menu.rs`, the portable menu model; `src/appkit`, the menu view) and `crates/cicada-app`
(`Contexts::host`, `WorkerReady::concurrency`). The menu model's tests; a manual check list next to the boards, light
and dark. Docs handoff: the app guide's menu bar section.
