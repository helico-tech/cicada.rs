# Brief: review a built change

The change loop asks you to review one OpenSpec change that a builder has built on a branch. The request
(`.reviews/<id>.md`) names the change and the commits: `base`, `head`, and from round 2 on `previous`, the head of the
round before. Its note holds the version and the builder's report. Read with git from the main checkout; you need no
checkout of your own.

## Read

- `AGENTS.md`.
- The change at `head`: `git ls-tree -r --name-only <head> openspec/changes/<change>` lists its files, and
  `git show <head>:<path>` prints one (proposal, design, specs, tasks, docs handoff). A fast-lane change (`Lane: fast`
  in the `## Lane` section of its proposal) has no design and no specs: its proposal and tasks are the intent you
  check the diff against.
- `git log --oneline <base>..<head>` and `git diff <base>..<head>`.
- Round 2 and later: only `git diff <previous>..<head>`, and the blocking findings of the round before in
  `.reviews/<change>.code.<round before>.result.md`. Check that each is fixed and that the fixes break nothing.

## Check

1. **Tasks.** Each task is ticked, and its work is in the diff.
2. **Spec.** Each scenario in the change's specs has a test. Name only the scenarios without one. A scenario about
   the loops themselves (briefs, config, the handoff) is proven by the builder's walkthrough and the existing checks,
   not by a test that repeats the prose.
3. **Bugs.** Wrong behaviour, an error path that is not handled, a race.
4. **Scope.** Behaviour that the design (for a fast-lane change: the proposal) does not describe.
5. **The rules of `AGENTS.md`.** A schema change is an appended migration, public items have doc comments,
   `CHANGELOG.md` says what changed, generated references are regenerated, and the version in `Cargo.toml`, the
   `cicada-version` of every skill in `skills/` and `.claude-plugin/plugin.json` is the one the note names.
6. **Docs.** The builder edits no protected docs (`README.md` beyond its release tag lines, `AGENTS.md`, `docs/` except
   ADRs and generated references). `docs-handoff.md` is there and true to the code at `head`: what users must learn,
   where it goes, caveats, Blocking, and a status that calls no prose written that is not. A protected file in the
   diff is allowed only as owner fixes the handoff names (commit and paths): it equals that commit
   (`git diff <commit> <head> -- <paths>` is empty) and holds no code.
7. **The builder's example and concepts** are true to the code.
8. **Boards.** A GUI change: read each board in `openspec/changes/<change>/mockup/` at `head` and the screen's code.
   Compare in both directions: an element of the board missing or changed in the code, or an element of the code the
   board does not show (sections, columns and their order, controls, texts, states such as empty, offline and error),
   is a fault unless the design lists it under `Departures from the board`. Sample data and the board's desktop
   background do not count, nor the system's dark colours for a board that `mockup/SOURCE.md` names as without dark
   colours.

Do not run the test suite: the builder ran the finishing checks and recorded gate evidence for its head (its report
says which), and `scripts/gate.py` requires that evidence again before main or a tag is pushed (ADR 0046). To prove a finding you may run one targeted test in a checkout of your own:
`git worktree add --detach .claude/worktrees/review-<change> <head>`, and `git worktree remove` it when you are done.

## What blocks

Only these start a fix round: wrong behaviour, a scenario without a test, a broken rule of `AGENTS.md`, a check that
fails, a missing or untrue docs handoff, a departure from a board that the design does not list. A failing check blocks
also when the handoff lists it under Blocking. Everything else is a note, at most five: wording, style, a better name,
something nice to have.

## Report

At most 300 words. Write it to a file outside the repository and hand it in with
`python3 scripts/review.py done <id> <file>`.

1. The first line: `Verdict: pass` or `Verdict: fix needed`.
2. Blocking findings, numbered. For each: file and line, what is wrong, how to see it, and what "fixed" looks like.
3. Notes.
4. What you did not check.
