# Proposal

## Why

An operator drains, undrains, declares gone and releases from the command line (request from the Factory session,
2026-11-25; the owner accepted the split (PL-SPLIT)).

## What Changes

- **No new commands.** `cicada-app` does not depend on `cicada-actors` (the app layer below the ready-made actors), and
  the other ready-made actors are operated through the generic `cicada send <actor> --type <Message> --payload <json>`
  and read through the actor read commands. A placement command would add that dependency for four messages that
  `send` already carries. So the operator path is documented recipes:
  `cicada send cicada.placement/runners --type Drain --payload '{"node": "mac-1"}'`, `Undrain`, `Gone`, and
  `Release` with `{"holder": …, "grant": …}`; the pool's state shows nodes, holders and the line.
- A test proves those exact payloads work over the HTTP send route the command uses, so the recipes cannot rot.

## Lane

Lane: fast

## Out of scope

A terminal UI page. Dedicated `cicada placement …` commands (a later change if operators ask; it needs the layering
question answered first).

## Order

After `placement-member`. No following change.

## Impact

A test in `crates/cicada-actors/tests` (or the HTTP tests) and a docs handoff with the recipes. No public API change.
