# Proposal

## Why

The owner found on 2026-11-27 that the inspector does not look like the mockup ([MOCK-FIX]). The Actors page, its
pane and its actions were built from words; nobody compared them with the board.

## What Changes

Match `mockup/InspectorActors.dc.html`: the toolbar (title with the actor count, context pop-up, Live pill, "Filter by
prefix", Send message, Pause and the pane toggle), the table (Address as a tree, Type, Version, Created, Status with a
dot), the footer ("16 of 1,284 shown · 3 running · 1 paused", "Updated 14:32:07"), and the pane: a header with icon,
name, "<type> · version N" and a status pill, the State / Mailbox N / Watches tabs, the Running turn group with Cancel
Turn, the Details group (Parent, Created, Pinned to, Paused), and Pause and Send Message… at the bottom. The design
starts from the owner's screenshot of the page today and lists every departure for the owner to agree.

## Order

After `inspector-overview-mockup`. Before `inspector-outcomes-mockup`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the Actors page's AppKit view and the pane). A manual check list next to the board, light
and dark. Docs handoff: the app guide's Actors text.
