# Proposal

## Why

The owner ran the inspector on 2026-11-27: the Overview "looks nothing like" the mockup ([MOCK-FIX]). The designs
described it in words; the builders never opened the board.

## What Changes

Match `mockup/InspectorOverview.dc.html`, and the sidebar and toolbar every board shares:

- Nodes as a table in a rounded group: Node (id over its label), Status with a dot, Running, Version, Last seen; "N
  online" beside the title. Today: one text line per node.
- Running Turns as a table beside Nodes (two columns): Actor, Message, Node, Time. Today: text, stacked.
- Types as a grid of four cards ("<n> actors · <m> nodes host it") with "Show All" to the Types page. Today: a full list
  of every type.
- Health as a pill in the toolbar ("Healthy" with a green dot). Today: its own section.
- Tiles as in the board, the failed tile linking to Outcomes ("Show outcomes").
- The sidebar: a count beside every item (Outcomes "N failed"), the label "Outcomes" (today "Outcome"), the board's
  selection style. The toolbar: no search field and no pane toggle on the Overview; the context pop-up with its dot.

The design lists what the API cannot give (for example a node's version, if `/nodes` lacks it) as departures.

## Order

After `tray-settings-mockup`. Before `inspector-actors-mockup`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the Overview model and its AppKit view, the sidebar and toolbar). Tests against an
in-process server; a manual check list next to the board, light and dark. Docs handoff: the app guide's Overview text.
