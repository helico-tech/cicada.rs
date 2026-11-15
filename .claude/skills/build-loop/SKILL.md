---
name: build-loop
description: Start or resume the build loop of this repository (ADR 0030). A subagent builds the first ready OpenSpec change in a worktree, the review loop reviews it through scripts/review.py, and the change lands and is released on the owner's yes. Runs in a session of its own, next to the design loop and the review loop.
disable-model-invocation: true
---

# The build loop

One of three loops, each in a session of its own (ADR 0030): the design loop describes changes with the owner, this
loop builds them, and the review loop reviews both. They hand over through files: a change with a `tasks.md` on `main`
is ready to build, and reviews go through `scripts/review.py`.

You orchestrate: you start the builder, ask for reviews, read reports, talk with the owner and land. You do not build
or review yourself.

## Start

Every time the skill is invoked, and again after a change has landed:

1. `git fetch origin`. If `origin/main` is ahead of `main`, tell the owner before anything is built on the old base.
2. Run `python3 scripts/status.py`, `python3 scripts/review.py list`, `git status --short` and `git worktree list`.
   The main checkout is on `main`. If the status lists problems, solve them with the owner first.
3. The `build:` line names the change to build. When it names none, run `python3 scripts/status.py --wait` in the
   background: it ends when the first change in the order is ready. Tell the owner in one line that you wait.

## Build

One change at a time: the first in the order, and only when it has a `tasks.md`.

1. **Earlier work.** If `git worktree list` shows the branch `feat/<change>` and you hold no agent for it, it is left
   from another session. A clean tree: `git worktree remove <path>`; the branch stays and the new builder picks it up.
   A dirty tree: ask the owner whether to continue it or drop it. To continue it, start the builder without isolation
   and add `Worktree: <path>` to its prompt.
2. **Version.** Read the version in `Cargo.toml` and the top heading of `CHANGELOG.md`. A heading that says
   `unreleased` is the version. Otherwise raise the minor number when the proposal marks something **BREAKING**, else
   the patch number.
3. **Builder.** Start it with the Agent tool: `subagent_type: "general-purpose"`, `model: "opus"`, `effort: "high"`,
   `isolation: "worktree"`. Prompt:

   > Read `<main checkout>/.claude/skills/build-loop/builder.md` and follow it. Change: `<change>`. Version: `X.Y.Z`.
   > Main checkout: `<main checkout>`.

   Keep its id, and tell the owner in one line.
4. **When the builder reports.** A question about the design goes to the owner; send the answer with SendMessage.
   `Docs: blocked` goes to Blocked on docs. Otherwise write `Version: X.Y.Z` and the builder's report to a file in your scratchpad, and ask for a review:

   ```sh
   python3 scripts/review.py request code <change> --head feat/<change> --note-file <that file>
   ```

   Run `python3 scripts/review.py wait <id>` in the background: it ends when the report is in.
5. **When the review reports.**
   - `Verdict: pass`: go to Present.
   - `Verdict: fix needed`: check each blocking finding against the code. Drop one only when it is plainly wrong, and
     tell the owner why. Send the rest to the builder. When it reports, ask for round 2 the same way: `request code`
     again numbers the round and names the previous head. One fix round: when round 2 still blocks, take what is stuck
     to the owner.
   - Notes do not go to the builder; they go to the owner in Present.

## Blocked on docs

The docs session owns `README.md`, `AGENTS.md` and the handwritten docs under `docs/` (AGENTS.md, Documentation
ownership; `protected()` in `scripts/docs-maintenance.py`). A builder never edits them; it writes what users must learn
into `openspec/changes/<change>/docs-handoff.md`. Most changes land with that handoff alone, and the docs session
writes the prose after they land. When a doctest, link, guarantee row or skill route cannot pass without new prose,
the builder reports `Docs: blocked`, and nothing lands until the prose is in the branch:

1. Write down the head (`git rev-parse feat/<change>`), the handoff and the paths of its Blocking items. Tell the owner,
   labelled: the docs session must prepare those paths on its branch against that head and current `main`, as whole
   files that keep its other edits, and name its commit and paths when done. It has no scan for unlanded branches yet,
   so ask explicitly. While nobody prepares them, the build stays blocked: no waiting it out, no disabled check, no
   hook bypass, no review.
2. When the commit comes, check it: `git diff <commit>^ <commit> --stat` and the diff itself hold only the named docs,
   no code, and both sides of a rename or deletion. Send the builder the commit and paths. It takes exactly those
   files into its branch without a merge, folds them into the commit that breaks the block, runs the blocked and the
   finishing checks and reports `Docs: owner fixes integrated`. Then ask for a review as in Build, step 4.
3. After every fix round or rebase, a clean one too, check again that each integrated path equals the docs branch
   (`git diff --quiet docs feat/<change> -- <paths>`) and that the prose still describes the code. Then the builder
   runs the affected checks in its worktree on the combined result, also when nothing changed:
   `cargo test -p cicada-examples --doc`, `--test docs` and `--test skill`. Unchanged prose can stop compiling against
   a new base. Only a changed path or behaviour needs a fresh preparation by the docs session first. A failing check
   sends the build back to step 1. A conflict in a protected file is never solved by taking one side: the docs session
   reconciles it.

## Present

Tell the owner, in this shape:

- What is new, in two or three sentences, and a small example: the builder's, at most 20 lines.
- New concepts and how each works. Leave this out when there are none.
- Results: tasks done, checks with their numbers, checks not run, what the review found and what was fixed.
- Docs: what the handoff asks of the docs session, and whether it is only handed off (the prose comes after landing)
  or owner fixes are integrated (name them, and the prose still to write). Never call handed-off prose written.
- The notes of the review, each with your pick: drop it, or a line in the later proposal that owns the area.
- Breaking changes, and the version.
- The question, labelled: "N. Land and release as `X.Y.Z`? yes / no".

## Land

Only on the owner's yes. A no goes back to the builder, or to the design loop.

1. `git fetch origin`. If `main` moved: `git -C <builder worktree> rebase main`. On a conflict, `git rebase --abort`,
   send the builder to rebase and resolve, and ask for one more review round. A branch with owner fixes integrated is
   checked again after any rebase, also a clean one: you check the paths, and the builder runs the affected checks in
   its worktree on the rebased branch (Blocked on docs, step 3). Merge only when they pass.
2. `git merge --ff-only feat/<change>` on `main`.
3. Move the change's reviews into its folder, so that they are archived with it:
   `mkdir -p openspec/changes/<change>/reviews && mv .reviews/<change>.* openspec/changes/<change>/reviews/`.
4. `openspec archive <change> -y`. A new spec in `openspec/specs/` gets one or two real sentences under `## Purpose`.
5. Release as in `docs/how-to/releasing.md`: the README tag lines and the changelog date in one commit
   `Release X.Y.Z: …` on `main`. The design loop shares this checkout and may have files open, so stage by path, never
   `git add -A` or `git commit -a`: `git add -A -- README.md CHANGELOG.md openspec/changes/<change>
   openspec/changes/archive openspec/specs`. Then, in the main checkout, `python3 scripts/gate.py release vX.Y.Z`
   (ADR 0046). It pins that commit, checks the version and dated notes, reuses or runs the gate evidence (the archive
   and release edits are classified again: Markdown alone gets fresh docs checks over the builder's full pass, anything
   else full checks) and the E2E suite, pushes `main` without force, tags, packages the skill, pushes the tag and
   publishes a verified draft Release. If it fails before the tag push, no tag exists: tell the owner. If it fails
   later, run it again: it resumes the same commit and tag object; never delete or move a pushed version tag.
   The pre-push hook must be installed once per repository, with the owner's yes:
   `python3 scripts/gate.py install-hook` in the main checkout; it refuses to replace another pre-push hook.
   An abandoned gate run (`python3 scripts/gate.py status` says so) goes to the owner before `recover`.
6. Clean up: `git worktree remove` the builder's worktree, `git branch -d feat/<change>`, delete the leftover
   `worktree-agent-…` branches, `git worktree prune`.
7. Tell the owner in two lines, with the path of the archived handoff
   (`openspec/changes/archive/<date>-<change>/docs-handoff.md`): the docs session writes its prose and refreshes the
   installation pins from it. Go to Start.

## Rules

- The owner's yes in Present covers the merge, the archive, the release commit, the push and the tag of that one
  change. Nothing else is pushed, merged or tagged.
- Subagents and reviewers never push, merge, tag or archive.
- While a change is built, only its builder edits its files, in its branch.
- Read an agent's worktree with `git -C <path>` and absolute paths; never `cd` into it.
- Tell the owner in one line when an agent or a review starts and when it reports. Never state a result before its
  report has arrived.
- A finding outside the change: a line in the later proposal that owns the area, or, when it blocks, a new change with
  only a `proposal.md` and an `Order` section. Ask the owner.
- When the session ends, name each worktree: kept because a build runs, or removed.
