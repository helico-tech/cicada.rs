Verdict: not yet

## Blocking

1. **Size.** Three behaviours (skip and end status, target watch, `Cancel`/`cancelType`), about 6–7 tasks. Split: `job-graph-failures` (decisions 1–5, 9 minus cancel) then `job-graph-cancel` (6–8, the `cancelled` checks). Or the owner accepts the size explicitly.

2. **Dispatch and its `Failed` for a cancelled job (spec gap).** jg's requirement "Each job message is sent in a turn of its own" checks only "current attempt not yet dispatched". Decision 7 needs "job running", but the spec does not modify that requirement. Worse: a `Dispatch` fails (gone target), `Cancel` runs, then the run's own `Failed` arrives; jg 5 ends that undispatched attempt `failed`, flipping `cancelled` to `failed` against decision 9. Fix: add a MODIFIED requirement so both apply only while the job is `running`, plus a scenario for this race.

## Notes

1. Decision 4 says "the run takes the result first" without a condition. Mailbox order is `(deliverAt, seq)` (G5). If the run's `Result` turn is rescheduled (`node`/`timeout`), it gets a later `deliverAt`, and the `Stopped` overtakes it: the job fails and the result is refused. Qualify it.
2. A `JobCancel` can reach the worker before its `JobStart` is handled, when that turn is rescheduled. Guide: a worker handles a cancel for an unstarted attempt.
3. A `Stopped` has no instance identity. If a target is deleted and recreated before a queued `Stopped` is handled, a newly dispatched job can fail by mistake. Rare; a guide line.
4. Decision 8, "a separate type if jg's code shares one", leaves an open choice in unbuilt code. Pick one now.
5. The `job-graph-retry` proposal still says "within the unreleased or next patch".

## Questions

Owner: split, or accept three behaviours in one change?

## Not checked

Builds, tests, most of brood, remote turns' `notification` flag.
