# Brief: review a built change

The build loop asks you to review one OpenSpec change that a builder has built on a branch. The request
(`.reviews/<id>.md`) names the change and the commits: `base`, `head`, and from round 2 on `previous`, the head of the
round before. Its note holds the version and the builder's report. Read with git from the main checkout; you need no
checkout of your own.

## Read

- `AGENTS.md`.
- The change at `head`: `git ls-tree -r --name-only <head> openspec/changes/<change>` lists its files, and
  `git show <head>:<path>` prints one (proposal, design, specs, tasks).
- `git log --oneline <base>..<head>` and `git diff <base>..<head>`.
- Round 2 and later: only `git diff <previous>..<head>`, and the blocking findings of the round before in
  `.reviews/<change>.code.<round before>.result.md`. Check that each is fixed and that the fixes break nothing.

## Check

1. **Tasks.** Each task is ticked, and its work is in the diff.
2. **Spec.** Each scenario in the change's specs has a test. Name only the scenarios without one.
3. **Bugs.** Wrong behaviour, an error path that is not handled, a race.
4. **Scope.** Behaviour that the design does not describe.
5. **The rules of `AGENTS.md`.** A schema change is an appended migration, public items have doc comments, the docs
   and `CHANGELOG.md` say what changed, generated references are regenerated, and the version in `Cargo.toml` is the
   one the note names.
6. **The builder's example and concepts** are true to the code.

Do not run the test suite: the builder ran the finishing checks (its report says which), and CI runs them before the
tag. To prove a finding you may run one targeted test in a checkout of your own:
`git worktree add --detach .claude/worktrees/review-<change> <head>`, and `git worktree remove` it when you are done.

## What blocks

Only these start a fix round: wrong behaviour, a scenario without a test, a broken rule of `AGENTS.md`, a check that
fails. Everything else is a note, at most five: wording, style, a better name, something nice to have.

## Report

At most 300 words. Write it to a file outside the repository and hand it in with
`python3 scripts/review.py done <id> <file>`.

1. The first line: `Verdict: pass` or `Verdict: fix needed`.
2. Blocking findings, numbered. For each: file and line, what is wrong, how to see it, and what "fixed" looks like.
3. Notes.
4. What you did not check.
