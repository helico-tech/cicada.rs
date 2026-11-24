# Design

Picks of the change loop, 2026-11-24, split from `job-graph-cancel` after its design review 1. Checked against the
`job-graph` design (agreed, not built), the core's watches and mailbox order (`docs/architecture.md`, Watch,
Notification, G5) and the reference (brood `crates/brood-core/src/pipeline/actor.rs`: `skip_dependents`, `settle`,
`ended`). ADR 0060 supplements ADR 0059. "jg N" is decision N of `job-graph`'s `design.md`.

## Decisions

1. **A failed, skipped or cancelled job skips its dependents, transitively.** After every change of a job, each
   waiting job with a need that is `failed`, `skipped` or `cancelled` becomes `skipped`, until nothing changes (a
   loop over at most 100 jobs, no recursion, as brood's `skip_dependents`), in the same commit. A skipped job has no
   attempt and no reason. This replaces jg 10's "dependents stay waiting". `cancelled` is in the rule now, so
   `job-graph-cancel` needs no change to it. The run still ends when nothing runs and nothing can start, so an ended
   run has no waiting job.
2. **The run's end status follows its jobs, as brood's `settle`**: `succeeded` when every job succeeded, else
   `failed` when a job failed, else `cancelled` (at least one job cancelled, none failed); the last is reached only
   once `job-graph-cancel` cancels jobs. Derived from the jobs alone, so the decoder checks it. Not "`cancelled` only
   after a whole-run `Cancel`": that needs a field remembering it.
3. **The run learns of a gone or failing worker by watching each job's target.** The `Dispatch` turn that sends a job
   message also watches its `to` (jg 4), in the same commit; the run unwatches a target when no running job of it has
   that target any more. Cicada tells only the watchers of an actor that a message to it failed, never the sender, so
   the run's watch on itself cannot see the job message fail. A `Failed` the store wrote about a target ends the job
   `failed` only when it is about that job's message: `from` the run, `messageType` the job's, and a payload that
   decodes as a `JobStart` of this run, this job and its current attempt, while the job is `running`. Reason: `the job
   message failed at the worker (<cause>): <error>`, cut to 2,000 characters. A `Stopped` about a target ends every
   running, dispatched job with that target `failed`, reason `the worker went away (<cause>)`. Other notifications
   and a client's are ignored. Not a worker-reported failure only: a handler that fails on the job message never
   reports. Not a failed-job-message signal without a watch: Cicada has none.
4. **What it cannot see**, and the guide says: work the worker handed on (out of scope); a worker that took the job
   message and then lost its outside work without failing a message, stopping or reporting; the failure of a later
   message of the worker. A target that hands work on and then stops itself fails its job: it must stay until the job
   reports. A `Stopped` does not say which instance stopped: a target deleted and created again before the run handles
   the `Stopped` fails a job dispatched to the new one (rare). A worker that reports and stops in one commit: the
   commit writes its sends before its notifications (architecture, Commit), so in the normal order the run takes the
   result first. Mailbox order is `(deliverAt, seq)` (G5): when the run's turn on the `Result` is rescheduled (cause
   `node` or `timeout`), the `Stopped` overtakes it, the job ends `failed` and the result is refused. A documented
   limit; `job-graph-retry` can run the job again.
5. **Watching a shared worker brings its other failures to the run**: one turn each, ignored. Throughput is not a
   goal, and the run unwatches a target when its jobs end. Not watching only per-job workers: the run cannot tell.
6. **Dispatch and its failure apply only while the job is `running`.** jg 4 sends a job message for "the current
   attempt not yet dispatched", and jg 5 fails such an attempt on a failed `Dispatch`; both now also need the job
   `running`. A client's `Result` can end an undispatched attempt first (the sender is not checked, jg 6), and
   `job-graph-cancel`'s `Cancel` will: the job must keep that status when its failed `Dispatch` is reported later.
7. **Ready jobs start after every change of a job's status**, not only on `Start` and after a result (jg's
   requirement): also after a failure from the watch (decision 3) and a failed dispatch (jg 5), and later after a
   `Cancel`, each freeing a slot. One rule in the handler: skip, start, then check the end, in that commit.
8. **The decoder also checks**: an ended run's status follows decision 2; a `skipped` job has a need that is failed,
   skipped or cancelled, and no attempt; no waiting job has such a need.

## ADR

ADR 0060 (with `job-graph-cancel`): skipped dependents, the end status, the target watch and what it cannot see.

## Contract and tests

No message, field or state shape changes, so the contract's hash stays; `docs/reference/contracts/cicada.job-graph.json`
is regenerated (`CICADA_UPDATE_CONTRACTS=true`) only if a doc comment changed. `wire_formats.rs` gains a sample state
with skipped jobs. `crates/cicada-actors/tests/job_graph.rs`, on virtual time with `TestActors`, covers every scenario
of the spec delta and updates `job-graph`'s test of a failed job, whose dependents now skip; unit tests cover the skip
loop, the end status and the decoder's new checks.

## Version

Additive: 0.20.8, the unreleased batch. The change loop ships `job-graph`, `job-graph-failures`, `job-graph-cancel`
and `job-graph-retry` in that one release, so no host sees two contract changes. No guarantee, core wire format,
schema or command line change; the end report's contract stays.

## Docs it breaks

None: the job graph's guide section is not written yet. For the docs handoff, an extension of the `## Job graph`
section in `docs/guide/actors.md`: skipped dependents; the run's end status; the watch on each target, what it sees
and what not (decision 4), that a target must stay until its job reports, that a `Stopped` does not say which
instance stopped, and that a rescheduled result turn can lose to a `Stopped`.
