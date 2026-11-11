---
name: change-loop
description: Start or resume this repository's way of working on OpenSpec changes (ADR 0020). A subagent builds the first ready change in a worktree in the background and a second one validates it, while the main session describes the next change together with the owner, one ahead.
disable-model-invocation: true
---

# The change loop

Two things run at once (ADR 0020):

- **Build**, in the background. A subagent builds the first change in the order, in its own worktree. A second agent
  validates it. You present the result to the owner, and on a yes you land and release it.
- **Describe**, in this session. You and the owner think the next change through. A fresh agent reviews it. Then you
  write its tasks.

You are the orchestrator: you talk with the owner, start agents, read their reports and decide what happens next. You
do not build or validate yourself.

## Start

Do this every time the skill is invoked, and again after a change has landed:

1. Run `python3 "${CLAUDE_SKILL_DIR}/status.py"`. It prints the changes in order, the state of each, what to build and
   what to describe. If it lists problems, solve them with the owner first. (The skill folder is
   `.claude/skills/change-loop` in the main checkout, should the variable not be filled in.)
2. Run `git status --short`, `git branch --show-current` and `git worktree list`. The main checkout must be on `main`.
   Uncommitted files that are not part of the change you describe are the owner's call.
3. Tell the owner in a few lines what is being built, what you will describe together, and what is in the way. Then
   start both loops.

States, from the files of a change: only `proposal.md` is *proposal*; with `design.md` or a spec it is *described*;
with `tasks.md` it is *ready*; with some tasks ticked *in build*; with all ticked *built*.

## Build

Build only the first change in the order, and only when it has a `tasks.md`. One build at a time.

1. **Earlier work.** If `git worktree list` shows the branch `feat/<change>` and you hold no agent for it, it is left
   from another session. A clean tree: `git worktree remove <path>` (the branch stays, the new builder picks it up).
   A dirty tree: ask the owner.
2. **Version.** Read the version in `Cargo.toml` and the top heading of `CHANGELOG.md`. A top heading that says
   `unreleased` is the version. Otherwise raise the minor number when the proposal marks something **BREAKING**, else
   the patch number (README, "breaking change").
3. **Builder.** Start it with the Agent tool: `subagent_type: "general-purpose"`, `model: "opus"`, `effort: "xhigh"`,
   `isolation: "worktree"`. Prompt:

   > Read `${CLAUDE_SKILL_DIR}/builder.md` and follow it. Change: `<change>`. Version: `X.Y.Z`. Main checkout:
   > `<path of the main checkout>`.

   Keep the agent's id. Tell the owner in one line, and go on describing.
4. **When the builder reports.** A question about the design goes to the owner; send the answer to the builder with
   SendMessage. Otherwise start the validator: `subagent_type: "general-purpose"`, `model: "fable"`,
   `isolation: "worktree"`. Prompt:

   > Read `${CLAUDE_SKILL_DIR}/validator.md` and follow it. Change: `<change>`. Version: `X.Y.Z`. The builder's
   > example and concepts: `<copied from its report>`.

5. **When the validator reports.** Read the findings. Drop one only when you checked the code and it is wrong. Send
   the rest to the builder with SendMessage. When the builder reports again, send the validator: "Round N: merge
   again and check again. The builder says: …". After three rounds without a pass, take what is stuck to the owner.
6. **Present.** When the validator passes, tell the owner, in this shape:
   - What is new, in two or three sentences.
   - A small example: the builder's, at most 20 lines.
   - New concepts and how each works. Leave this out when there are none.
   - Results: tasks done, checks with their numbers, checks not run, and what the validation rounds found and fixed.
   - Breaking changes, and the version.
   - The question: land and release as `X.Y.Z`?
7. **Land**, only on the owner's yes. A no goes back to the builder, or to describing.
   1. If `main` moved: `git -C <builder worktree> rebase main` (`git worktree list` shows the path next to
      `feat/<change>`). On a conflict, `git rebase --abort`, send the builder to rebase and resolve, then run one more
      validation round.
   2. `git merge --ff-only feat/<change>` on `main`.
   3. `openspec archive <change> -y`. If it made a new spec in `openspec/specs/`, replace the placeholder under
      `## Purpose` with one or two real sentences.
   4. Release as in `docs/operations.md`, Releasing, steps 1 to 5: the README tag lines, the changelog date, one
      commit `Release X.Y.Z: …` that holds the archive, push `main`, wait for CI (`gh run watch`),
      `scripts/release-check.sh vX.Y.Z`, tag, push the tag. If CI or the check fails: no tag, tell the owner.
   5. Clean up: `git worktree remove` the builder's and the validator's worktrees, `git branch -d feat/<change>`,
      delete the leftover `worktree-agent-…` branches, `git worktree prune`.
   6. Tell the owner in two lines, and run Start again.

## Describe

Describe only the change that `status.py` names. Never go further than one change ahead of the one being built.

1. **Read** its proposal, the ADRs it names, the code it touches, and the proposals before and after it. Documents
   written ahead of its turn (a design, a spec, `draft-tasks.md`) are a draft: go through them with the owner.
2. **Think it through with the owner**, one topic at a time. Start from the proposal's "To decide in the design" list
   and add what you found yourself. For each decision:
   - the options, one line each: what it adds and what it costs;
   - a small example per option where it helps: the code a user writes, the request, the table;
   - your pick, why, and why not the others. Bring your own ideas. Say so when something is more than the problem
     needs.
   Ask a few questions per message, not all at once. Write each decision into `design.md` when it is made, so a new
   session can go on from there.
3. **Write** the rest of `design.md` and the spec deltas in `specs/<capability>/spec.md` when you both think it is
   thought through. `openspec validate <change> --strict` must pass. Do not write `tasks.md` yet.
4. **Review.** Start a fresh agent: `subagent_type: "general-purpose"`, `model: "fable"`, no isolation (it must see
   the uncommitted files). Prompt, and nothing more, so that it is not led by your reasoning:

   > Read `${CLAUDE_SKILL_DIR}/reviewer.md` and follow it. Change: `<change>`.

5. **Bring the findings to the owner**, each with your own view. Decide together and update the design and the spec.
   Review again when the design changed much.
6. **Tasks**, when the owner, you and the review agree. Write `tasks.md` by the rules in `openspec/config.yaml`; the
   last task runs the finishing checks. Validate again. Commit the change's folder (and its ADR, if it has one) on
   `main` as `Ready <change>: <what it does>`. Do not push; the next release pushes `main`.
7. If nothing is being built and this change is the first in the order, start its build. If a build is running, stop
   describing here: the change after this one waits until the build has landed.

## Rules

- The owner's yes in Present covers the merge, the archive, the release commit, the push and the tag of that one
  change. Nothing else is pushed, merged or tagged.
- Subagents never push, merge, tag or archive.
- While a change is being built, only its builder edits its files, in its branch.
- Tell the owner in one line when an agent starts and when it reports. Never state an agent's result before its
  report has arrived.
- A finding outside the scope of a change becomes a new change with only a `proposal.md` and an `Order` section. Ask
  the owner where it goes in the order.
- When a session ends, name the worktrees: kept because a build is running, or removed.

## A new session

The state is in the files. `status.py` shows it. A branch `feat/<change>` with ticked tasks is a build that was under
way: follow Build, step 1. A change with a `design.md` and no `tasks.md` is a description that was under way: read the
decisions in its `design.md` and go on from Describe, step 2.
