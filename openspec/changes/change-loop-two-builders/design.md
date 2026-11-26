# Design

The owner wants the tray and inspector built next to the placement work, by a second builder on another provider's
model, and every change reviewed by a model other than its builder's (2026-11-26). Decisions taken with the owner
(CB-SHARE, CB-GATE, CB-FALLBACK, CB-MODEL: all the picks below).

## Decisions

1. **Two builders when no crate is shared, declared in the proposal (CB-SHARE).** Every proposal's `## Impact` gets a
   line `Crates: <names>` (or `Crates: none` for process and docs changes). `scripts/status.py` reads it and names up
   to two changes on its `build:` line: the first ready change in the order, and the next ready one whose crates do not
   meet the first's (or that touches none). A ready change without the line is built alone, as today. Shared files
   (`Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, the ADR index, the skills' versions) do not count: they are merged
   at landing. Not taken: inferring crates from `tasks.md` (paths are named loosely).
2. **One gate run at a time (CB-GATE).** The gate's lock stays: a second builder's gate waits for the first, in short
   tries, then runs under its own 2-hour limit (the memory note on gate timeouts). Not taken: parallel gate runs (two
   gate worktrees; parallel compiling slowed every run and caused timeouts on 2026-11-26).
3. **The second builder on Codex, by name (CB-MODEL).** The second builder runs on `gpt-6.2-sol` at reasoning effort
   `high`, started with `delegate_task` in a worktree (`isolation: worktree`, `target: codex`), as the harness's rules
   require for another provider. The first builder stays a Claude subagent. Which change gets which builder: the first
   in the order gets Claude, the second gets GPT, unless the owner says otherwise.
4. **Reviews by the other model.** A change built by Claude is reviewed by a fresh GPT reviewer (`delegate_task` on
   Codex, the same model, no worktree); one built by GPT by a fresh Claude reviewer subagent. `scripts/review.py
   request` takes `--built-by NAME`, keeps it in the request, and `take` refuses a reviewer whose `--by` names the same
   model family; `list` shows both.
5. **Codex unavailable: build with Claude, review waits (CB-FALLBACK).** When `delegate_task` to Codex fails, the loop
   builds the second change with a Claude subagent; its review still goes to GPT and waits until Codex answers again.
6. **Still one session, landing one at a time.** The session describes, lands and releases. Before landing, a change is
   rebased on `main`, its ADR number checked (renumbered when another change took it), and the gate runs on the landed
   tip, as today. A third builder is never started.
7. **An ADR.** ADR 0073 supersedes the "one builder at a time" rule of ADR 0055 (whose status line notes it); the rest
   of ADR 0055 stands. `.claude/skills/change-loop/SKILL.md` and `builder.md` follow; `AGENTS.md`'s "Specs and changes"
   and the release guide's change-loop section go to the docs session.

## Tests

`scripts/test_status.py`: two ready changes with disjoint `Crates:` lines are both named on `build:`; with a shared
crate only the first; a change without the line is built alone; `Crates: none` pairs with anything. `scripts/
test_review.py`: `--built-by` is kept and listed; `take` by the same model family is refused.

## Docs

The handoff gives `AGENTS.md` ("Specs and changes", Flow) and `docs/how-to/releasing.md` the two-builder rule, the
`Crates:` line, and reviews by the other model.
