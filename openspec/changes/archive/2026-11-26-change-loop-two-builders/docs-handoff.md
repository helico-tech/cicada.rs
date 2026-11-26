# Docs handoff for change-loop-two-builders

Version: none. This change touches no crate, consumer skill or wire format, so it lands without a version bump,
changelog entry or release, and the installation pins stay as they are.

## What changed

Users of the library see nothing new. The way this repository is worked on changes (ADR 0073, superseding the "one
builder at a time" rule of ADR 0055):

- **Two builders.** The change loop runs at most two builders at once, when their changes share no crate. Every
  proposal's `## Impact` starts with a line `Crates: <names>` (`Crates: none` for process and docs changes); a change
  without it is built only alone. Shared workspace files (`Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, the ADR index,
  the skills' versions) do not count: they are merged at landing. `openspec/config.yaml` states the rule; its line
  "Throughput is not a goal" became "Two builders when changes share no crate (ADR 0073); simplicity still comes
  first".
- **`python3 scripts/status.py`** names up to two changes on its `build:` line: those in build (in build, built, or
  ready with a worktree), and for a free slot the first ready change in the order whose crates meet none in build. Two
  changes in build that share a crate are listed under `problems:`. An `Order` line that names an archived change is
  resolved through that change's own `Order`, so changes may land out of order and the open changes stay one line.
  Proved by `BuildLineTest` in `scripts/test_status.py`.
- **Builders on two models.** The first builder is a Claude subagent; the second runs on GPT (`gpt-6.2-sol`, effort
  `high`) through Codex, started in T3 Code with `delegate_task` in its own worktree, on the branch `feat/<change>`.
  When Codex is unavailable, Claude builds the second change.
- **Reviews by the other model.** A change built by Claude is reviewed by GPT, one built by GPT by Claude.
  `python3 scripts/review.py request code` needs `--built-by NAME`, and `take` of a code review needs `--by NAME`;
  each name starts with its family, `claude` or `gpt` (`codex` and `openai` count as `gpt`). `take ID` refuses a
  reviewer of the builder's family, `take` without an id skips such requests, and `list` shows builder and reviewer.
  When Codex is down for a whole request, a fresh Claude reviewer takes it with `--same-family "Codex unavailable"`,
  which is recorded in the taken marker, at the end of the report and in `list`. Design reviews have no builder:
  either model may take them. Proved by `FamilyTest` in `scripts/test_review.py`.
- **One gate run at a time.** The gate keeps its single lock. Whoever runs it (a builder, or the session at landing
  and release) retries `busy` in short tries for at most 50 minutes, and the try that gets the lock runs under a
  2-hour limit, never stopped once started.
- Still one session: it describes, lands and releases, one change at a time. Before landing, a change is rebased on
  `main` and its ADR number checked (renumbered when another change took it); the gate runs on the landed tip.
- `.claude/skills/change-loop/SKILL.md`, `.claude/skills/change-loop/builder.md` and the `--by` names of
  `.claude/skills/review-loop/SKILL.md` follow these rules.

## 1. AGENTS.md: the flow

**Where.** `AGENTS.md`, `Specs and changes: OpenSpec`: the intro line "Work on them goes through one change loop
(ADR 0055)." and the items "Every proposal has an `Order` section" and `Flow:`.
**Text.**
- Intro: "Work on them goes through one change loop (ADR 0055; two builders by ADR 0073)."
- After the `Order` item: "Every proposal's `## Impact` starts with `Crates: <names>`, or `Crates: none` for process
  and docs changes. `status.py` names up to two changes to build, whose crates meet nowhere; a change without the line
  is built only alone."
- Flow: "One session, `/change-loop`, describes the next change with the owner, has up to two builders build changes
  that share no crate, each in its own worktree (a Claude subagent, and GPT through Codex), has a fresh reviewer of
  the other model review each change, lands the changes on `main` one at a time and in any order, and releases the
  batch when it is time. Reviews go through `scripts/review.py`, which keeps requests in `.reviews/`, records who built
  a change and refuses a reviewer of the same model family; another harness can take one with `/review-loop`
  (`.claude/skills/review-loop/SKILL.md`). …" (the rest of the item as it is).
**Caveats.** The second builder's worktree is made by T3 Code's `delegate_task`, so it may lie outside
`.claude/worktrees/`; say "in its own worktree" rather than naming the folder for both.

## 2. docs/how-to/releasing.md: landing and the gate's lock

**Where.** `docs/how-to/releasing.md`, `Land a change`, and `Interrupted or busy runs`.
**Text.**
- `Land a change`: "Up to two builders may run at once on changes that share no crate (ADR 0073, linked as
  `../adr/0073-two-builders-in-the-change-loop.md`). The change loop lands them one at a time, in the
  order they pass review: it rebases each on `main`, checks its ADR number, and runs the gate on the landed tip."
- `Interrupted or busy runs`, after "A busy gate refuses another run; wait for its owner to finish.": "Builders and the
  change loop retry `busy` in short tries for at most 50 minutes; each try runs as a background command with a
  2-hour limit, since the try that gets the lock is the run. Never stop a run once it started."
**Caveats.** Gate checks and the lock itself did not change.

## Blocking

None.

## Status

`python3 -m unittest discover -s scripts -p 'test_*.py'`, `openspec validate change-loop-two-builders --strict` and
`python3 scripts/status.py` (two builds, no problem) pass; see the build report for the numbers. Items 1 and 2 are
proposed text; no prose is written yet.
