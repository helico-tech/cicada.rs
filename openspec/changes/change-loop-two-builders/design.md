# Design

The owner wants the tray and inspector built next to the placement work, by a second builder on another provider's
model, and every change reviewed by a model other than its builder's (2026-11-26). Decisions taken with the owner
(CB-SHARE, CB-GATE, CB-FALLBACK, CB-MODEL: all the picks below).

## Decisions

1. **Two builders when no crate is shared, declared in the proposal (CB-SHARE).** Every proposal's `## Impact` gets a
   line `Crates: <names>` (or `Crates: none` for process and docs changes); `openspec/config.yaml` states the rule, and
   its line "Throughput is not a goal" gives way to "two builders when changes share no crate (ADR 0073); simplicity
   still comes first". This change adds the line to every open proposal. **The build rule:** changes in build fill
   the two builder slots; a free slot goes to the first ready change in the order whose crates meet none of the
   changes in build (a ready change that shares a crate is passed over by a later one that does not). A ready change
   without the line is built only when no other build runs. `status.py` names up to two changes on its `build:` line
   (in build or to start) and lists a problem when two changes in build share a crate. Shared files (`Cargo.toml`,
   `Cargo.lock`, `CHANGELOG.md`, the ADR index, the skills' versions) do not count: they are merged at landing. A
   crate that depends on the other's (core and tray) can still clash: the gate run on the landed tip finds it. Not
   taken: inferring crates from `tasks.md` (paths are named loosely).
2. **One gate run at a time (CB-GATE).** The gate's lock stays: `gate.py run` fails with "busy" while another run holds
   it, so whoever runs it (a builder, or the session at landing and release) retries in short tries for at most 50
   minutes, and the try that gets the lock runs under a 2-hour limit. Not taken: parallel gate runs (two
   gate worktrees; parallel compiling slowed every run and caused timeouts on 2026-11-26).
3. **The second builder on Codex, by name (CB-MODEL).** The second builder runs on `gpt-6.2-sol` at reasoning effort
   `high`, started with `delegate_task` in a worktree (`isolation: worktree`, `target: { providerInstanceId: codex,
   model: gpt-6.2-sol, options: { reasoningEffort: high } }`), as the harness's rules require for another provider; it
   works on the branch `feat/<change>`, so `status.py` finds it. The first builder stays a Claude subagent. Which change
   gets which builder: the first
   in the order gets Claude, the second gets GPT, unless the owner says otherwise.
4. **Reviews by the other model.** A change built by Claude is reviewed by a fresh GPT reviewer (`delegate_task` on
   Codex, the same model, no worktree); one built by GPT by a fresh Claude reviewer subagent. `scripts/review.py
   request code` takes `--built-by NAME`; names (`--built-by`, `--by`) start with their family, `claude` or `gpt`
   (`codex` and `openai` count as `gpt`), and `--by` no longer defaults to `unknown` for a code review. `take ID`
   refuses a reviewer of the builder's family; `take` without an id skips such requests and takes the next it may.
   `list` shows builder and reviewer. Design reviews have no builder: either family may take them.
5. **Codex unavailable: Claude builds, and a same-family review is recorded (CB-FALLBACK).** When `delegate_task` to
   Codex fails, the loop builds the second change with a Claude subagent. A review that should go to GPT and cannot
   (Codex down for its whole request) is taken by a fresh Claude reviewer with `--same-family "Codex unavailable"`,
   which `review.py` records in the result; the session tells the owner. So landings do not stop while Codex is down.
6. **Still one session, landing one at a time, in any order.** The session describes, lands and releases. Before
   landing, a change is rebased on `main`, its ADR number checked (renumbered when another change took it), and the
   gate runs on the landed tip, as today. Changes may land out of order: `status.py` resolves an `## Order` line that
   names an archived change through that change's own `Order`, so the open changes stay one line (design review,
   blocking 1). A third builder is never started.
7. **An ADR.** ADR 0073 supersedes the "one builder at a time" rule of ADR 0055 (whose status line notes it); the rest
   of ADR 0055 stands. `.claude/skills/change-loop/SKILL.md` and `builder.md` follow; `AGENTS.md`'s "Specs and changes"
   and the release guide's change-loop section go to the docs session.

## Tests

`scripts/test_status.py`: a change in build and a ready change with disjoint `Crates:` lines are both named on
`build:`; with a shared crate the ready one is passed over by a later disjoint one; a change without the line is built
only alone; `Crates: none` pairs with anything; two changes in build that share a crate are a problem; an `Order` naming
an archived change resolves through it. `scripts/
test_review.py`: `--built-by` is kept and listed; `take ID` by the builder's family is refused and `take` without an id
skips it; `--same-family` is recorded; names without a family are refused for code reviews.

## Docs

The handoff gives `AGENTS.md` ("Specs and changes", Flow) and `docs/how-to/releasing.md` the two-builder rule, the
`Crates:` line, and reviews by the other model.
