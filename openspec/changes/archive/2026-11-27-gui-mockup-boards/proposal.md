# Proposal

## Why

The owner ran the tray and the inspector on 2026-11-27 and found that they "look nothing like what you proposed". The
designs named the owner's mockup (https://claude.ai/artifact/S49tsmjjgjuVTZQuqkaNQH) but described the screens in
words; the builders never opened it, `tray-menu-bar`'s design chose plain native menu items without saying that this
departs from the mockup, and no review compared a screen with it. The owner agreed that GUI changes are built and
reviewed against their mockup board ([MOCK-RULE], 2026-11-27).

## What Changes

- A change that adds or changes a screen (the tray menu, the Settings window, an inspector page) copies the board it
  follows from the mockup into its folder as `mockup/<Board>.dc.html`, names it in the design, and names it in the task
  that builds the screen ("match board X").
- The design lists every place where the screen departs from the board, each with its reason, for the owner to agree;
  nothing else may depart.
- The builder reads the board before building and reports each departure it found necessary; the code reviewer checks
  the screen's code against the board (layout, sections, columns, controls, texts) and blocks on an unagreed departure.
- The change's manual check list starts with "next to board X"; the owner's look at it is part of the release check.

## Order

After `inspector-message-actions`. Before the mockup fix changes (`tray-menu-mockup`, `inspector-overview-mockup` and
the ones for the other pages), which follow the new rule.

## Impact

Crates: none.

`.claude/skills/change-loop/SKILL.md` and `builder.md`, `.claude/skills/review-loop/design.md` and `code.md`, and
`openspec/config.yaml`'s rules for designs and tasks. No ADR: it adds a check to the existing loop (ADR 0055) without
changing who does what. Docs handoff: none beyond the skills themselves.
