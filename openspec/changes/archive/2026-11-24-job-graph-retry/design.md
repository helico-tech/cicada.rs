# Design

Picks of the change loop, 2026-11-24, on the proposal, checked against the landed `job-graph` code
(`crates/cicada-actors/src/job_graph.rs`: `advance`, `take_result`, `check_job`, `report`), the `job-graph-failures`
and `job-graph-cancel` designs (ready, not landed) and the reference (brood `crates/brood-core/src/pipeline/actor.rs`:
`Retry`, `reopen`). ADR 0061. "jg N", "jf N" and "jc N" are decisions of `job-graph`, `job-graph-failures` and
`job-graph-cancel`.

## Decisions

1. **`Retry { job }`, public, puts one failed or cancelled job back to `waiting`**, and the normal start (jf 7) runs
   in the `Retry` commit: the job starts there with the next attempt number when its needs succeeded and a slot is
   free, else later like any waiting job. No start path of its own, as brood's `reopen` + `advance`: the in-flight
   limit and the needs stay one rule. Its `output` is cleared; its old attempts stay (jg 8 keeps at most 20, dropping
   the oldest at each start). The end report's `reason` of a job is its last attempt's only when that attempt's outcome
   is the job's status, else null: a retried job cancelled before it starts shows no old failure reason.
2. **Its skipped dependents come back by recomputing the skips**: in the `Retry` commit every `skipped` job becomes
   `waiting`, then jf 1's skip loop runs on the current statuses. Jobs skipped because of another failed or cancelled
   job are skipped again; those skipped only because of the retried job stay `waiting`. A skipped job has no attempt
   and no reason (jf 1), so nothing is lost. Not tracking who skipped whom: one more field and invariant.
3. **A retry of an ended run reopens it**: status `running`, `endedAt` null, in the `Retry` commit; it ends again by
   the usual rule (jf 2) and reports that end. While running again, a `Report` sends nothing (jg 11).
4. **Each end has a number, so a report target can tell a repeat from a new end.** The state counts the run's ends
   (`ends`, a `u32` like the attempt numbers, 0 until the first); each end raises it, and `JobGraphEnd` carries
   `end`, the number of that end. A `Report`
   sent again repeats the current number; a retried run's next end has a higher one. The target keeps the report with
   the highest `end` per run and treats one with the same number as a repeat. Reports may arrive out of order (a
   target's turn that is rescheduled, G5), so the latest arrival is not the latest end. This adds a field to the end
   report's contract, in the same 0.20.8 release as `job-graph`, so no host sees it change. Not the store time of the
   end: the store's clock can stand still (ADR 0028). Not the attempt numbers: a retry can end with no new attempt.
   Numbers can have gaps: an end whose `Report` had not run before a `Retry` reopened the run is never reported (the
   `Report` finds the run running). The highest number wins. This refines jg 11's "a repeat for the same run is one".
5. **A retry is refused for good, state unchanged**, for an unknown job or a name out of bounds; a job that is not
   `failed` or `cancelled` (also a second `Retry` of the same job, now `waiting` or `running`); a job with a need that
   is `failed`, `skipped` or `cancelled`, which would be skipped at once ("retry its need first"); a job whose last
   attempt number is the largest (`u32::MAX`); and an ended run whose `ends` is the largest. There is no cap on retries:
   the 20-attempt bound is on kept records, and a host's own policy caps retries (automatic policies are out of scope).
6. **Results and notices of a replaced attempt stay refused or ignored, with no new code**: a `Result` must name a
   running job's current attempt (jg 6–7), a `Failed` about a job message must match the current attempt (jf 3), and
   an old `Dispatch` finds its attempt no longer current (jg 4). A cancel still reaches a replaced attempt's worker:
   `SendCancel` acts on any kept, dispatched attempt closed `cancelled`, whatever the job's status (jc 2), so a
   `Retry` right after a `Cancel` does not leave the old worker running. A `Stopped` about the target still fails a
   running retried job on that target (jf 4): it names no instance.
7. **The decoder's status rules change for a job with old attempts**: `waiting` and `cancelled` allow a last attempt
   that is closed `failed` or `cancelled` (a retried job waiting, or cancelled again before it started); `running`
   needs an open last attempt; `succeeded` and `failed` a last attempt closed so; `skipped` none. This replaces jg's
   "waiting has no attempt", and relaxes jc 4: a `cancelled` job's last attempt may also be closed `failed`
   (cancelled again before it started). An ended run has `ends` of 1 or more. A started job's needs all
   succeeded (jg 12) still holds: only failed and cancelled jobs are retried, and a succeeded need never changes.

Wire: state `{"maxInFlight":2,…,"status":"running","endedAt":null,"ends":1,"jobs":[{"name":"test","status":"waiting",
"output":null,"attempts":[{"attempt":1,…,"outcome":"failed","reason":"exit 1"}]}]}`; end report `{"run":"run-7",
"end":2,"status":"succeeded","jobs":[…]}`. `ends` absent: 0.

## Spec

A delta on the archived capability `job-graph`. `job-graph-failures` and `job-graph-cancel` are archived first, so
the MODIFIED texts repeat their latest versions: "A job graph run under a fixed type name" and "A bad graph is refused
before the run exists" from `job-graph-cancel`'s delta (commit ca2ab0b), "The end is reported to the report target"
from `openspec/specs/job-graph/spec.md` (neither of the others changes it).

## ADR

ADR 0061 (written with this design): `Retry` and the end number of the report, a wire field hosts depend on; it
refines ADR 0059's "a repeat for the same run is one" into "the highest end number per run wins".

## Contract and tests

The contract reference `docs/reference/contracts/cicada.job-graph.json` is regenerated: `Retry`, `ends`, and `end` in
`JobGraphEnd`. `wire_formats.rs` gains samples of `Retry`, a reopened state and an end report with `end`.
`crates/cicada-actors/tests/job_graph.rs`, on virtual time with `TestActors`, covers every scenario of the spec delta;
unit tests cover the recomputed skips and the decoder's changed rules.

## Version

Additive: 0.20.8, the unreleased batch, shipped with `job-graph`, `job-graph-failures` and `job-graph-cancel`. No
guarantee, core wire format, schema or command line change.

## Docs it breaks

None: no guide, example or skill route names the job graph yet. A guide block written from `job-graph`'s handoff
that builds a `JobGraphEnd` needs the `end` field. For the docs handoff, an extension of the `## Job
graph` section in `docs/guide/actors.md`: `Retry` of a failed or cancelled job and when it is refused; skipped
dependents coming back; a reopened run and its new end; keep the report with the highest `end` per run, and expect
gaps in the numbers; a retried job cancelled before it starts has no reason in the report; a worker keys a `JobCancel`
and its work by attempt, as a cancel of a replaced attempt can arrive after the retry.
