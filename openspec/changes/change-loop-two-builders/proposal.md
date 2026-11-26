# Proposal

## Why

The line holds two kinds of work that touch different code: the placement changes (core, actors, app) and the tray
and inspector changes (two new crates). With one builder at a time (ADR 0055), the tray and inspector wait behind
the placement work even when nothing is shared, and builds and gate runs are the slow part. The owner wants a second
builder on another model, GPT-6.2 through Codex, in its own worktree, and every change reviewed by a model other
than its builder's (2026-11-26).

## What Changes

- A new ADR that supersedes the "one builder at a time" part of ADR 0055:
  - The change loop may run two builders at once when their changes touch no crate in common. Shared files
    (`Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, the ADR index) don't count; they are merged at landing.
  - The second builder may use another provider's model. In T3 Code it is started with `delegate_task` and a
    worktree, as the harness's rules say; elsewhere the loop says how.
  - Each change is reviewed by a model other than the one that built it: Claude reviews GPT's builds, GPT reviews
    Claude's. `scripts/review.py` records who built and who reviewed.
- Still one session: it describes, lands and releases, one change at a time. Before landing, the change is rebased
  on `main`, its ADR number is checked (renumbered if taken meanwhile), and the gate runs on the landed tip.
- `scripts/status.py` names up to two changes on its `build:` line, and flags two builds that share a crate.
- `.claude/skills/change-loop/SKILL.md` and `builder.md` follow the new rules.

## To decide

- How the loop tells that two changes share no crate: from each `tasks.md`, or a line in the proposal.
- Whether two gate runs may run at once on one machine, or the second waits for the first.
- What the loop does when the other provider is not available: wait, or build with its own model.

## Out of scope

- A second change-loop session.
- More than two builders.
- Changes to the gate's checks.

## Order

After `placement-resource-key`, before `tray-worker-control`. First of the changes not yet described, because it
changes how the loop builds the rest.

## Impact

A new ADR and its row; ADR 0055's status line; `.claude/skills/change-loop/`; `scripts/status.py`,
`scripts/review.py` and their tests. No crate and no release. Docs handoff: the change-loop section of
`docs/how-to/releasing.md` and `AGENTS.md`'s "Specs and changes" text, for the docs session.
