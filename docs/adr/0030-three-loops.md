# 0030. Three loops: design, build and review

Date: 2026-11-13

## Status

Accepted. Supersedes [ADR 0020](0020-the-change-loop.md). Partly superseded by [ADR 0055](0055-one-change-loop-a-fast-lane-and-batched-releases.md):
the three sessions, every landed change a release, and the full loop for every change.

## Context

ADR 0020 put describing and building in one session: the main session described the next change with the owner,
while a subagent built the change before it and a second subagent validated the build. On 11 and 12 November that loop
landed 18 changes, and it was slow. The transcripts of those builds show:

- A change took 2.5 to 4 hours from the start of its build to its release; one took 6 hours.
- Changes were large: 11 to 22 tasks, a design of 150 to 550 lines, a spec of 90 to 700 lines, a diff of 1,500 to
  7,600 lines.
- A builder worked 40 to 110 minutes. About half of it was the model thinking and writing at effort xhigh; the other
  half was commands, mostly the whole test suite, run again and again.
- Every change needed at least one validation round, and one needed six. A round was 15 to 30 minutes of validator (40
  to 110 on Fable), then a fix, then all checks again. Many findings were small: wording, comments over 120 columns.
- A design was reviewed two to six times, 8 to 10 minutes each.

Compiling was not the cause: a fresh worktree builds the workspace in about a minute. The owner wants quicker
iterations: smaller changes and faster reviews, no Fable for reviews, and reviews that another agent harness than
Claude Code can run.

## Decision

Work on changes goes through three loops, each in a session of its own.

- **Design** (`.claude/skills/design-loop`, started with `/design-loop`). The main session describes the next change
  with the owner, has it reviewed, writes its tasks and commits it on `main` as ready. It may run ahead until two ready
  changes wait for the build loop.
- **Build** (`.claude/skills/build-loop`, `/build-loop`). A subagent (Opus, effort high) builds the first ready change
  in a worktree under `.claude/worktrees/`, on the branch `feat/<change>`. The session asks for a code review, sends
  the blocking findings to the builder for one fix round, presents the change, and on the owner's yes lands and
  releases it. When round 2 still blocks, the owner decides.
- **Review** (`.claude/skills/review-loop`, `/review-loop`, or any other harness). It takes review requests, reviews
  each one, and hands in a report. Its briefs name no harness and no model.

The loops hand over through files. `scripts/status.py` reads the state of each change from its files, as before.
`scripts/review.py` keeps the review requests in `.reviews/` in the main checkout, which git ignores: a request, a
claim that only one reviewer can make, and a report. When a change lands, its reviews move into its folder and are
archived with it. We do not use A2A: it needs every harness to run an agent server, and files need nothing.

Lighter work:

- **Small changes.** One behaviour, at most 5 tasks plus the finishing checks, a design of about 80 lines. A bigger
  proposal is split into a line of changes (`openspec/config.yaml`).
- **Targeted checks.** The builder runs the tests of what it touches while it works, and the whole workspace once, at
  the end.
- **Reviews that read.** The code reviewer reads the diff against the tasks and the spec, and does not run the suite
  again: the builder ran the finishing checks, and CI runs them before the tag. Only wrong behaviour, a scenario
  without a test, a broken rule of `AGENTS.md` or a failing check blocks; the rest are notes for the owner. A report
  has at most 300 words.
- **One design review**, and a second only when the design changed much.

Unchanged from ADR 0020: one order of changes, the states of a change, `tasks.md` written last, every landed change a
release, and who may do what. Subagents and reviewers never push, merge, tag or archive; the build session does, and
only after the owner's yes for that change.

## Consequences

- Two sessions commit on `main` in one checkout: the design loop its `Ready` commits, the build loop its releases.
  Each stages its own paths, never everything.
- The owner talks with two sessions: the design loop for decisions, the build loop for the yes to land.
- More, smaller releases.
- The code review trusts the builder's report on the checks. A check that the builder skipped or got wrong shows in CI
  before the tag, not in the review.
- A reviewer in another harness needs only a shell, `git` and `python3`. AGENTS.md points it to the review loop.
- The link check of the docs skips `.reviews/`.
- The builder's model and effort are in the build loop's skill and change with the models.
