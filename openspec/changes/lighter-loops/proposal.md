# Proposal

## Why

The process costs more than the code. From 2026-11-15 to 2026-11-22, `main` got 13 releases. Even
`node-view-rustdoc`, a rustdoc-only change, went proposal → ready → build → tick → release → pin refresh: about six
commits, a full gate with E2E, a tag and a docs merge. Most of that time goes to the steps every change must take,
not to the change itself.

Two rules cause most of it:

- Every change goes through the full loop, whatever its kind (ADR 0030, from ADR 0020).
- Every landed change is a release (ADR 0020, kept by ADR 0030). Each release also brings a "Refresh installation
  pins" commit and a docs merge.

## What Changes

- **A fast lane** for changes that do not change behaviour: rustdoc and comments, tests, internal refactors that
  leave the public API, wire formats, schema and guarantees alone. Such a change needs no `design.md`, spec deltas
  or `tasks.md`: one proposal, one build, one code review, one commit.
- **Batched releases.** A landed change waits on `main` under `unreleased` in the changelog. The build loop releases
  when the line has no ready change left, or sooner when a landed change is urgent (a security fix, a bug a consumer
  hits). One release can hold several changes.
- A new ADR that supersedes these two rules of ADR 0030, and the loop skills, `openspec/config.yaml`,
  `scripts/status.py` and the `AGENTS.md` lines that state them.

To decide in the design:

- What exactly counts for the fast lane, and who decides: the proposal says so and the design loop agrees, or the
  build loop may move a change there.
- Whether a fast-lane change still gets a code review, or only the builder's checks.
- When a batch is released: when the line is empty, after N changes, once a day, or on the owner's word.
- Whether the gate runs E2E on every landed change or only before a release.
- How `scripts/status.py` shows changes that are landed but not released.

## Out of scope

Changing the gate's checks themselves, the review briefs, the docs handoff (ADR 0045) or the rules for what a
release must pass. Any change to the library's behaviour, API, wire format, schema or guarantees.

## Order

After `node-view-rustdoc` (released as 0.20.5), before `dependency-advisories`. It goes first because it makes every
change after it cheaper.

## Impact

A new ADR and its row in `docs/adr/README.md`; `.claude/skills/design-loop`, `build-loop` and `review-loop`;
`openspec/config.yaml`; `scripts/status.py` and its tests. The `AGENTS.md` lines on releases go to the docs session
in the docs handoff. Documentation and process only, so no library release.
