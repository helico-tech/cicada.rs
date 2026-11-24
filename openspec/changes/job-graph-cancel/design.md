# Design

Picks of the change loop, 2026-11-24, on the proposal, updated after design reviews 1 and 2. Review 1 split the
skipped dependents, the end status and the watch on each target into `job-graph-failures`; this change is `Cancel`
and `cancelType` only. Checked against the `job-graph` and `job-graph-failures` designs and the reference (brood
`crates/brood-core/src/pipeline/actor.rs`: `Cancel`, `cancel_job`). ADR 0060 supplements ADR 0059. "jg N" is
decision N of `job-graph`'s `design.md`, "jf N" of `job-graph-failures`'s.

## Decisions

1. **`Cancel { job? }`, public**: one job, or every job when `job` is absent. A waiting job becomes `cancelled` at
   once. A running job becomes `cancelled` in the same commit, its attempt ends with outcome `cancelled` and reason
   `cancelled`, and its target is unwatched (jf 3) unless another running job has it. A job that has ended or is
   skipped, and a run that has ended, are left alone: a repeated `Cancel` changes nothing. An unknown job name, or
   one out of bounds, is refused for good, state unchanged. Then jf 7 applies in that commit: dependents skip (jf 1),
   ready jobs start in the freed slots, and the run may end, `cancelled` when no job failed (jf 2).
2. **A running worker is told through a cancel message type per job**: a new optional field `cancelType` (1–200
   characters, no NUL, checked by the constructor and the decoder). For a cancelled job whose attempt was dispatched
   and that has a `cancelType`, the `Cancel` commit sends the run an internal `SendCancel {job, attempt}`; only one
   the run sent itself, while that job is `cancelled` with that last attempt, sends `cancelType` to `to` with
   `JobCancel {run, job, attempt}` and the crate's shared retry policy. A worker's family takes it as e.g.
   `CancelBuild(JobCancel)`. The job is `cancelled` from the `Cancel`, not from the notice: in the same commit, a gone
   target would reject the `Cancel` itself. A failed `SendCancel` changes nothing. A job not yet dispatched is not
   told, and its `Dispatch` sends nothing (jf 6); one whose failed `Dispatch` is reported after the `Cancel` stays
   `cancelled` (jf 6). A job without `cancelType` is not told; its outside work goes on, and its late result is
   refused (jg 7).
3. **The attempt outcome is its own type in the state, `succeeded | failed | cancelled`** (picked in review 1); the
   worker's `Result` keeps its outcome type `succeeded | failed`, so a worker cannot report `cancelled`.
   `JobGraphEnd` names the attempt by its number (jg 11), so the end report's contract does not change.
4. **The decoder also checks**: a `cancelled` job's last attempt, when it has one, is closed with outcome
   `cancelled` (an open one is refused); an attempt with outcome `cancelled` belongs to a `cancelled` job.

Wire, a job: `{"name":"build",...,"cancelType":"CancelBuild","status":"cancelled","attempts":[{"attempt":1,
"dispatchedAt":"…","endedAt":"…","outcome":"cancelled","reason":"cancelled"}]}`; `cancelType` absent or null: none.

## ADR

ADR 0060 (with `job-graph-failures`), for this change: `Cancel` and `cancelType`.

## Contract and tests

The contract reference `docs/reference/contracts/cicada.job-graph.json` is regenerated
(`CICADA_UPDATE_CONTRACTS=true`): `Cancel`, `SendCancel`, `cancelType` and the attempt outcome. `wire_formats.rs`
gains samples of `Cancel`, `SendCancel`, `JobCancel` and a state with cancelled jobs; the `JobGraphEnd` schema does
not change. `crates/cicada-actors/tests/job_graph.rs`, on virtual time with `TestActors`, covers every scenario of
the spec delta; unit tests cover the `cancelType` bounds and the decoder's new checks.

## Version

Additive: 0.20.8, the unreleased batch. The change loop ships `job-graph`, `job-graph-failures`, `job-graph-cancel`
and `job-graph-retry` in that one release, so no host sees two contract changes. No guarantee, core wire format,
schema or command line change; the end report's contract stays.

## Docs it breaks

None: the job graph's guide section is not written yet. For the docs handoff, an extension of the `## Job graph`
section in `docs/guide/actors.md`: `Cancel` of one job or all, and the run's `cancelled` end; `cancelType` with the
`CancelBuild(JobCancel)` variant, and that a worker without one is not told; that a worker can get `JobCancel` before
its `JobStart` (when its turn on the job message is rescheduled) and then ignores that attempt's job message; and that
a host on an older contract of `cicada.job-graph` follows "Changing a contract"
(`docs/operations.md#changing-a-contract`).
