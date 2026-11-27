# Design

GUI changes followed the mockup only through the words of their designs, and those words lost most of it (owner,
2026-11-27, [MOCK-RULE]). The board itself must travel with the change, so the builder and the reviewer see what the
owner drew.

## Decisions

1. **Every screen named, the board in the change.** The design of a change that adds or changes a screen names each such
   screen (the tray menu in a state, the Settings window, an inspector page, a sheet) with its board or "no board". It
   copies each board (the artboard's `.dc.html`, read with the Artifact tool) into `openspec/changes/<change>/mockup/`,
   with `mockup/SOURCE.md` naming the artifact's URL, its version and the date of the copy. The board is a snapshot:
   before landing, the loop reads the live board again; if it changed, the change does not land on the standing yes
   until the owner either accepts the copied version or the change is updated to the new copy and reviewed against it.
   Only the board's HTML counts: the boards carry their own light and dark colours; a board without a dark variant is
   named so, and the system's dark colours are then no departure. The building task says "match board X". The loop
   session copies the boards, as only it has the Artifact tool; builders read the files.
2. **No board, or departures, decided by the owner.** For a screen with no board the owner either has one drawn first or
   agrees in the design to build it without one, following the nearest board's style (named). The design has a section
   "Departures from the board": each place where a screen will differ or add something the board does not show (a
   control, a section, a state) (a native control that cannot look the same, data the API does not have), with its
   reason. The owner agrees to them with the design; a departure found while building goes back to the owner as a
   question, never decided by the builder. Sample data and the board's desktop background are not departures.
3. **Reviews compare.** The design reviewer checks that every screen is named with a board or an agreed "no board", and
   that every element of each board is either in the design or under Departures. The code reviewer reads the board and
   the screen's code and blocks, in both directions, on an element missing, changed or added without an agreed
   departure: sections, columns and their order, controls, texts and states (empty, offline, error).
4. **The owner looks.** The manual check list's first item is "next to board X, light and dark"; a GUI change is done
   in the release check only when the owner has seen it. Builders still never drive the GUI (no `osascript`, no screen
   capture).
5. **Where it lives.** The change-loop skill (Describe and the builder's prompt), `builder.md`, the review briefs and
   `openspec/config.yaml`'s rules for designs and tasks. No code, no ADR.

## Tests

The skill tests stay green (`cargo test -p cicada-examples --test skill`, `--test docs`); the loop's own rules are
proven by the next GUI change following them (`tray-menu-mockup`).
