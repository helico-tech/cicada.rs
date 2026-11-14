---
name: design-loop
description: Start or resume the design loop of this repository (ADR 0030). The main session describes the next OpenSpec change together with the owner, has the review loop review it through scripts/review.py, writes its tasks and commits it as ready. Runs in a session of its own, next to the build loop and the review loop.
disable-model-invocation: true
---

# The design loop

One of three loops, each in a session of its own (ADR 0030). This one describes changes together with the owner, so
that the build loop always has a ready change. The review loop reviews what you write; you ask for a review through
`scripts/review.py`. You never build.

## Start

Every time the skill is invoked, and again after a change is ready:

1. `git fetch origin`. If `origin/main` is ahead of `main`, tell the owner.
2. Run `python3 scripts/status.py`, `python3 scripts/review.py list` and `git status --short`. If the status lists
   problems, solve them with the owner first.
3. The `describe:` line names the change to describe. It names none while two ready changes wait for the build loop:
   tell the owner, and stop until one of them is being built.

## Small changes

A change is small: one behaviour, at most 5 tasks plus the finishing checks, a design of about 80 lines. Check this
before anything else. A bigger proposal is split into a line of small changes: propose the split to the owner, each
piece with one line on what it does, and on a yes write their proposals, each with an `Order` section. Then describe
the first piece.

## Describe

1. **Read** the proposal, the ADRs it names, the code it touches, and the proposals before and after it. Documents
   written ahead of their turn (a design, a spec, `draft-tasks.md`) are a draft: go through them with the owner.
2. **Think it through with the owner**, a few topics per message. Start from the proposal's "To decide" list and add
   what you found yourself. For each decision:
   - the options, one line each: what it adds and what it costs;
   - a small example per option where it helps: the code a user writes, the request, the table;
   - your pick, why, and why not the others. Say so when something is more than the problem needs.

   Label every question, yes/no ones too, with labels that are unique in the message, and end with the labels and
   your picks in one line. Write each decision into `design.md` when it is made, so that a new session can go on.
3. **Write** the rest of `design.md` (the decisions, each with its reason and the options not taken in a line) and the
   spec deltas in `specs/<capability>/spec.md`. Name the doc blocks, links, guarantee rows and skill routes the change
   breaks: the builder cannot fix them, as the docs session owns that prose (AGENTS.md, Documentation ownership).
   `openspec validate <change> --strict` passes. No `tasks.md` yet.
4. **Review.** `python3 scripts/review.py request design <change>`, then run `python3 scripts/review.py wait <id>` in
   the background: it ends when the report is in. Tell the owner in one line. One round; a second only when the design
   changed much.
5. **Bring the findings to the owner**, each with your own view. Decide together, and update the design and the spec.
6. **Tasks**, when the owner agrees. Write `tasks.md` by the rules in `openspec/config.yaml`; the last task runs the
   finishing checks. A task never asks the builder to edit `README.md`, `AGENTS.md` or the handwritten docs: it asks
   for that content in `docs-handoff.md` and keeps the checks; a block the design names as broken gets a fix prepared
   by the docs session before landing. Validate again. Commit on `main` as `Ready <change>: <what it does>`: only the
   change's folder, and its ADR with its row in `docs/adr/README.md` if it has one. The build loop shares this checkout, so stage by
   path, never `git add -A` or `git commit -a`. Do not push; the next release pushes `main`.
7. Go to Start.

## Rules

- You never build, merge, push, tag or archive. You commit only `Ready` commits and the proposals the owner agrees to.
- One change at a time is described, and never more than two ready changes wait for the build loop.
- A change being built belongs to its builder: do not edit its files on `main`.
- An idea or a finding outside the change becomes a change with only a `proposal.md` and an `Order` section, or a line
  in the later proposal that owns the area. Ask the owner where it goes.
