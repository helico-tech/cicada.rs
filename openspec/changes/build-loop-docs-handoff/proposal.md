# Proposal

## Why

Since 2026-11-14 the `docs` session owns README and the hand-written docs (AGENTS.md, Documentation ownership;
`docs/maintaining/maintenance.md`). A local hook refuses other worktrees' commits of those files. The loops still
send builders into that prose:

- `.claude/skills/build-loop/builder.md` says nothing about it.
- The code review brief (`.claude/skills/review-loop/code.md`, rule 5) asks that "the docs ... say what changed".
- `openspec/config.yaml` has tasks keep the doctests of README and the guides compiling. The design loop writes
  guide and README tasks into `tasks.md` (e.g. task 5 of `deep-failure-notices`).

`cicada-agent-skill` hit this: the hook refused its review fixes to README and the core guide. Its prose went to the
docs session as `openspec/changes/cicada-agent-skill/docs-handoff.md` (owner decision 20a). Every later builder that
touches user-facing behaviour will hit the same wall.

## What Changes

- The builder brief: leave README and hand-written docs alone; write what users must learn into
  `openspec/changes/<change>/docs-handoff.md` (what changed, where it belongs, the substance), committed with the
  change. ADRs, generated references, the changelog and README release pins stay with the builder.
- The design loop and `openspec/config.yaml`: tasks ask for a docs handoff instead of guide or README edits.
- The code review brief: check the handoff instead of the prose.
- The build loop: when a change lands, point the docs session at its archived handoff.
- Release pins: `docs/how-to/releasing.md` step 1 has the release update the dependency examples in
  `docs/how-to/install.md`, but the hook refuses that file from the main checkout (0.17.2 left them on v0.17.1 for the
  docs session). Make the procedure and the hook agree on who moves those pins.
- Decide how a public API change keeps the doctests of README and the guides compiling when the builder may not edit
  them. The docs session follows main every 10 minutes, so for a while main can fail those doctests.

## Out of scope

The docs session's own procedure and hooks; access isolation; rewriting earlier archived changes.

## Order

After `cicada-agent-skill`. Before `usage-error-password-redaction`.

## Impact

`.claude/skills/build-loop/` (SKILL.md, builder.md), `.claude/skills/design-loop/`, `.claude/skills/review-loop/code.md`,
`openspec/config.yaml`, and AGENTS.md's loop paragraph if the docs session agrees. No library code, API, wire format,
schema or guarantee changes.
