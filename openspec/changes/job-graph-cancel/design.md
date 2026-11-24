# Design

Picks of the change loop, 2026-11-24, on the proposal, checked against the `job-graph` design (agreed, not built), the
core's watches (`docs/architecture.md`, Watch and Notification) and the reference (brood
`crates/brood-core/src/pipeline/actor.rs`: `skip_dependents`, `settle`, `Cancel`). ADR 0060 supplements ADR 0059,
which is not edited. Numbers refer to the decisions of `job-graph`'s `design.md` where they say "jg".

## Decisions

1. **A failed, skipped or cancelled job skips its dependents, transitively.** After every change of a job, each
   waiting job with a need that is `failed`, `skipped` or `cancelled` becomes `skipped`, until nothing changes (a
   loop over at most 100 jobs, no recursion, as brood's `skip_dependents`), in the same commit. A skipped job has no
   attempt and no reason. This replaces jg 10's "dependents stay waiting". The run still ends when nothing runs and
   nothing can start, so an ended run has no waiting job.
2. **The run's end status follows its jobs, as brood's `settle`**: `succeeded` when every job succeeded, else
   `failed` when a job failed, else `cancelled` (at least one job cancelled, none failed). So a whole-run `Cancel`
   ends the run `cancelled` unless a job had already failed; a cancel of one job does too once the rest succeeded or
   skipped. The status is derived from the jobs alone, so the decoder checks it. Not "`cancelled` only after a
   whole-run `Cancel`": that needs a field remembering it. `cancelled` is in jg's run status set already.
3. **The run learns of a gone or failing worker by watching each job's target.** The `Dispatch` turn that sends a job
   message also watches its `to` (jg 4), in the same commit; the run unwatches a target when no running job of it has
   that target any more. Cicada tells only the watchers of an actor that a message to it failed, never the sender, so
   the run's watch on itself cannot see the job message fail. A `Failed` the store wrote about a target ends the job
   `failed` only when it is about that job's message: `from` the run, `messageType` the job's, and a payload that
   decodes as a `JobStart` of this run, this job and its current attempt. Reason: `the job message failed at the
   worker (<cause>): <error>`, cut to 2,000 characters. A `Stopped` about a target ends every running, dispatched job
   with that target `failed`, reason `the worker went away (<cause>)`. Other notifications and a client's are
   ignored. Not a worker-reported failure only: a handler that fails on the job message never reports. Not a
   failed-job-message signal without a watch: Cicada has none.
4. **What it cannot see**, and the guide says: work the worker handed on (out of scope); a worker that took the job
   message and then lost its outside work without failing a message, stopping or reporting; the failure of a later
   message of the worker. A target that hands work on and then stops itself fails its job: it must stay until the job
   reports. A worker that reports and stops in one commit: the commit writes its sends before its notifications
   (architecture, Commit), so the run takes the result first; a test pins it.
5. **Watching a shared worker brings its other failures to the run**: one turn each, ignored. Throughput is not a
   goal, and the run unwatches a target when its jobs end. Not watching only per-job workers: the run cannot tell.
6. **`Cancel { job? }`, public**: one job, or every job when `job` is absent. A waiting job becomes `cancelled` at
   once. A running job becomes `cancelled` in the same commit, its attempt ends with outcome `cancelled` and reason
   `cancelled`, and its target is unwatched (decision 3). A job that has ended or is skipped, and a run that has
   ended, are left alone: a repeated `Cancel` changes nothing. An unknown job name, or one out of bounds, is refused
   for good, state unchanged. Then decisions 1 and 2 apply: dependents skip, ready jobs start, the run may end.
7. **A running worker is told through a cancel message type per job**: a new optional field `cancelType` (1–200
   characters, no NUL, checked by the constructor and the decoder). For a cancelled job whose attempt was dispatched
   and that has a `cancelType`, the `Cancel` commit sends the run an internal `SendCancel {job, attempt}`; only one
   the run sent itself, while that job is `cancelled` with that last attempt, sends `cancelType` to `to` with
   `JobCancel {run, job, attempt}` and the crate's shared retry policy. A worker's family takes it as e.g.
   `CancelBuild(JobCancel)`. The job is `cancelled` from the `Cancel`, not from the notice: in the same commit, a gone
   target would reject the `Cancel` itself. A failed `SendCancel` changes nothing. A job not yet dispatched is not
   told: its `Dispatch` then finds it not running and sends nothing. A job without `cancelType` is not told; its
   outside work goes on, and its late result is refused (jg 7).
8. **The attempt's outcome gains `cancelled`**, in the state only: the `Result`'s outcome stays `succeeded | failed`
   (a separate type if jg's code shares one), and `JobGraphEnd` names the attempt by its number (jg 11), so the end
   report's contract does not change. The run's own contract changes: `Cancel`, `SendCancel`, `cancelType`, the
   outcome. A host that ran `job-graph`'s release goes through "Changing a contract".
9. **The decoder also checks**: an ended run's status follows decision 2; a `skipped` job has a need that is failed,
   skipped or cancelled, and no attempt; no waiting job has such a need; a `cancelled` job's last attempt, when it has
   one, ended `cancelled`; an attempt with outcome `cancelled` belongs to a `cancelled` job.

Wire, a job: `{"name":"build",...,"cancelType":"CancelBuild","status":"cancelled","attempts":[{"attempt":1,
"dispatchedAt":"…","endedAt":"…","outcome":"cancelled","reason":"cancelled"}]}`; `cancelType` absent or null: none.

## ADR

ADR 0060, written with this design: skipped dependents, the end status, the target watch and what it cannot see,
`Cancel` and `cancelType`. ADR 0059 stays as accepted.

## Contract and tests

The contract reference `docs/reference/contracts/cicada.job-graph.json` is regenerated
(`CICADA_UPDATE_CONTRACTS=true`) and `wire_formats.rs` gains samples of `Cancel`, `SendCancel`, `JobCancel` and a
state with skipped and cancelled jobs. `crates/cicada-actors/tests/job_graph.rs`, on virtual time with `TestActors`,
covers every scenario of the spec; unit tests cover the skip loop and the decoder's new checks.

## Version

Additive: the next patch, taken at the build from `python3 scripts/status.py`. No guarantee, core wire format, schema
or command line change; the end report's contract stays.

## Docs it breaks

None: the job graph's guide section is not written yet. For the docs handoff, an extension of the `## Job graph`
section in `docs/guide/actors.md`: skipped dependents; the watch on each target, what it sees and what not (decision
4), and that a target must stay until its job reports; `Cancel` of one job or all; `cancelType` with the
`CancelBuild(JobCancel)` variant and that a worker without one is not told; the run's end status (decision 2).
