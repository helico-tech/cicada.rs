# 0010. OpenSpec for work tracking

Date: 2026-11-09

## Status

Accepted

## Context

Work was tracked in `work/`: a queue, epics with stories, and issues. Other Helico projects (karakuri) use
[OpenSpec](https://github.com/Fission-AI/OpenSpec), and one way of working across them is simpler.

## Decision

Changes, designs and tasks live in `openspec/`. `work/` is gone; there is one tracker.

- Open issues became changes with only a `proposal.md`, marked "proposal only". Specs, design and tasks are written
  when the change is next. New findings out of scope are recorded the same way.
- Finished epics and issues moved, as they were, to `openspec/changes/archive/2026-11-09-work-tracker-history/`.
- `docs/architecture.md` stays the reference for behaviour, and ADRs stay the record of decisions. A change to a
  guarantee or a wire format still needs an ADR.
- `openspec/specs/` starts empty and grows as changes are archived. We do not rewrite `docs/architecture.md` as specs.

## Consequences

- There is no ordered queue any more; `openspec list` shows what is open, and the choice of what is next is made when
  work starts.
- Until a capability has a spec, a change adds its requirements as new ones. For a while, behaviour is described partly
  in `docs/architecture.md` and partly in `openspec/specs/`; the architecture document wins if they differ.
