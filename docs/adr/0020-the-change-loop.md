# 0020. The change loop

Date: 2026-11-11

## Status

Superseded by [ADR 0030](0030-three-loops.md).

## Context

ADR 0010 moved all work into OpenSpec and dropped the queue: what was next was chosen when work started. ADR 0018 then
planned eleven changes that must land in a fixed order, and ADR 0019 added one more. A change was described and built
in one go, by one session. The owner saw the design only after the code existed. The flow that `AGENTS.md` named
(`/opsx:` commands) was never installed in this repository.

We want two things at once: building runs in the background, and the owner and the main session think the next change
through together before anyone builds it.

## Decision

Work on changes goes through one loop, held in the skill `.claude/skills/change-loop` and started with `/change-loop`.

- **One order.** Every open change has an `Order` section in its proposal that names the change before it. The open
  changes form one line. This replaces "there is no ordered queue" in ADR 0010.
- **States.** A change is a proposal only, then described (design and spec), then ready (it has a `tasks.md`), then
  built, then archived. `tasks.md` is written last. Its existence means: agreed, ready to build.
- **Describe, one ahead.** The main session describes a change together with the owner: it shows alternatives with
  examples, gives its own view, and asks for decisions. Only the first change that is not ready is described, and
  never further than one change ahead of the one being built. Before the tasks are written, a fresh agent reviews the
  design for uncertainty, scope that is not needed, and trouble later. The tasks are written when the owner, the main
  session and the review agree. The ready change is committed on `main`.
- **Build, in the background.** Only the first change in the order is built, and only when it is ready. A subagent
  (Opus, effort xhigh) builds it in a worktree of its own under `.claude/worktrees/`, on the branch `feat/<change>`. A
  second agent (Fable) validates the committed result on a clean checkout. The main session sends the findings back to
  the builder. After three rounds without a pass, the owner decides.
- **Land, on the owner's yes.** The main session presents the built change: the results, a small example of what is
  new, and how any new concept works. On a yes it puts the branch on `main` (fast-forward), archives the change,
  releases it as the next version (`docs/operations.md`, Releasing) and pushes. Every landed change is a release.
- **Who may do what.** Subagents never push, merge, tag or archive. The main session does, and only after the owner's
  yes for that change.

## Consequences

- `main` moves while a change is built, because ready changes are committed there. A branch is rebased on `main`
  before it lands.
- There are more and smaller releases: one per change.
- A worktree has its own `target/` folder, so the first build in it compiles everything again.
- A change described ahead of its turn is not ready. `child-actors` keeps its drafted tasks as `draft-tasks.md` until
  its turn comes.
- A new finding is still a change with only a `proposal.md` (ADR 0010), now with an `Order` section. Where it goes in
  the line is the owner's call.
- The model names (Opus for building, Fable for validating and reviewing) are in the skill and change with the models.
- The rest of ADR 0010 stands: OpenSpec is the one tracker, and `docs/architecture.md` wins over `openspec/specs/`.
