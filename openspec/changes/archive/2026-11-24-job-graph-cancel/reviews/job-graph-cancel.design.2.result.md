Verdict: not yet

## Blocking

1. **design.md was not narrowed after the split.** Decisions 1–5 and the first three checks of 9 repeat `job-graph-failures` 1–5 and 7. Decision 4 contradicts failures 4: it says "the run takes the result first" unconditionally, without the caveats on rescheduling and `Stopped` instances. "ADR", "Contract and tests" (skip-loop tests) and "Docs it breaks" (skips, watch) also still name failures' work: builder and docs session get it twice. Fix: keep 6–8, the two cancelled checks of 9 and the wire example; point to failures for the rest.

2. **Ready jobs do not start after a `Cancel` (spec gap).** Decision 6 says "ready jobs start". jg's "Ready jobs start up to the in-flight limit" starts jobs only "on `Start` and after every result it takes", unchanged here. Fix: MODIFY it to "after every change of a job" and add a scenario: limit 1, `a` runs, `b` is independent, `Cancel a` → `b` starts in that commit.

## Notes

1. Decision 8 still says "a separate type if jg's code shares one" (round 1). Pick: the state's outcome gets its own enum.
2. The docs handoff misses two points from ADR 0060: a worker may get `JobCancel` before its `JobStart`, and the "Changing a contract" steps for hosts (upgrading guide).
3. Scenario "Inconsistent cancelled jobs" does not cover a cancelled job whose last attempt is still open.
4. `job-graph-failures` has the same gap as blocking 2: a `Failed` or `Stopped` frees a slot, but nothing starts.

## Questions

Owner: will `job-graph`, failures and cancel ship in one release? If not, a host that ran `job-graph` meets a second contract change. Putting `Cancel`, `cancelType` and the `cancelled` outcome in `job-graph`'s contract avoids it.

## Not checked

Builds, tests, brood, `openspec validate`.
