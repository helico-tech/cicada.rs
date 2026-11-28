# Proposal

## Why

The owner found on 2026-11-27 that the inspector does not look like the mockup ([MOCK-FIX]). The Outcomes page and
its message pane were built from words; nobody compared them with the board. The pages without a board (Messages,
Running Turns, Nodes, Types, Watches) follow the same look.

## What Changes

Match `mockup/InspectorOutcomes.dc.html` (the Outcomes list and a failed message's pane, with the message actions of
`inspector-message-actions`), and give the pages without a board the boards' table, footer and pane style. The design
starts from the owner's screenshots of the pages today and lists every departure for the owner to agree.

## Order

After `inspector-actors-mockup`. Before `worker-view`.

## Impact

Crates: cicada-inspector.

`crates/cicada-inspector` (the AppKit views of the Outcomes page and the other list pages). A manual check list next to
the board, light and dark. Docs handoff: the app guide's page texts, if they change.
