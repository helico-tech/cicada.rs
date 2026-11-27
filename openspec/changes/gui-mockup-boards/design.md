# Design

GUI changes followed the mockup only through the words of their designs, and those words lost most of it (owner,
2026-11-27, [MOCK-RULE]). The board itself must travel with the change, so the builder and the reviewer see what the
owner drew.

## Decisions

1. **The board in the change.** A change that adds or changes a screen copies each board it follows (the artboard's
   `.dc.html` from the mockup artifact, read with the Artifact tool) into `openspec/changes/<change>/mockup/`. The board
   is a snapshot: a later edit to the mockup reaches a change only when the change loop copies it again. The design
   names the boards; the task that builds the screen says "match board X". The loop session copies the boards, as only
   it has the Artifact tool; builders read the files.
2. **Departures are named.** The design has a section "Departures from the board": each place where the screen will
   differ (a native control that cannot look the same, data the API does not have), with its reason. The owner agrees
   to them with the design; a departure found while building goes back to the owner as a question, never decided by the
   builder. Sample data and the board's desktop background are not departures.
3. **Reviews compare.** The design reviewer checks that every element of the board is either in the design or under
   Departures. The code reviewer reads the board and the screen's code and blocks on an element missing or changed
   without an agreed departure: sections, columns and their order, controls, texts and states (empty, offline, error).
4. **The owner looks.** The manual check list's first item is "next to board X, light and dark"; a GUI change is done
   in the release check only when the owner has seen it. Builders still never drive the GUI (no `osascript`, no screen
   capture).
5. **Where it lives.** The change-loop skill (Describe and the builder's prompt), `builder.md`, the review briefs and
   `openspec/config.yaml`'s rules for designs and tasks. No code, no ADR.

## Tests

The skill tests stay green (`cargo test -p cicada-examples --test skill`, `--test docs`); the loop's own rules are
proven by the next GUI change following them (`tray-menu-mockup`).
