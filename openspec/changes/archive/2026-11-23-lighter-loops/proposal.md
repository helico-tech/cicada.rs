# Proposal

## Why

The process costs more than the code. From 2026-11-15 to 2026-11-22, `main` got 13 releases. Even
`node-view-rustdoc`, a rustdoc-only change, went proposal → ready → build → tick → release → pin refresh: about six
commits, a full gate with E2E, a tag and a docs merge. Most of that time goes to the steps every change must take,
not to the change itself.

Three rules cause most of it:

- Every change goes through the full loop, whatever its kind (ADR 0030, from ADR 0020).
- Every landed change is a release (ADR 0020, kept by ADR 0030). Each release also brings a "Refresh installation
  pins" commit and a docs merge.
- Design, build and review run in three sessions of their own (ADR 0030). The owner keeps two or three sessions going,
  and a request waits in `.reviews/` until someone opens a review session.

## What Changes

- **One session, the change loop.** It describes a change with the owner, starts a builder subagent in a worktree
  and, for each review, a fresh reviewer subagent; it lands and releases. While a builder runs, it describes the next
  change. `/change-loop` replaces `/design-loop` and `/build-loop`; reviews still go through `scripts/review.py`, so
  another harness can still take one.
- **A fast lane** for changes that do not change behaviour: rustdoc and comments, tests, internal refactors that
  leave the public API, wire formats, schema and guarantees alone. Such a change needs no `design.md`, spec deltas
  or design review: a proposal, short tasks, one build and one code review.
- **Batched releases.** A landed change waits on `main` under `unreleased` in the changelog. The change loop releases
  when the line has no ready change left, or sooner when a landed change is urgent (a security fix, a bug a consumer
  hits). One release can hold several changes.
- A new ADR that supersedes these three rules of ADR 0030, and the loop skills, `openspec/config.yaml`,
  `scripts/status.py` and the `AGENTS.md` lines that state them.

Decided in `design.md`: what counts for the fast lane, its review, when a batch is released, where E2E runs, how
`scripts/status.py` shows unreleased work, and what happens when E2E fails at a release.

## Out of scope

Changing the gate's checks themselves, the review briefs, the docs handoff (ADR 0045) or the rules for what a
release must pass. Any change to the library's behaviour, API, wire format, schema or guarantees.

## Order

After `node-view-rustdoc` (released as 0.20.5), before `dependency-advisories`. It goes first because it makes every
change after it cheaper.

## Impact

A new ADR and its row in `docs/adr/README.md`; `.claude/skills/change-loop` in place of `design-loop` and
`build-loop`, and `review-loop`; `openspec/config.yaml`; `scripts/status.py`, `scripts/review.py` and their tests. The
`AGENTS.md` lines on the loops and releases go to the docs session in the docs handoff; the moved `builder.md` link
blocks landing until it prepares them. Documentation and process only, so no library release.
