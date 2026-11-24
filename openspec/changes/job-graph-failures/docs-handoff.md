# Docs handoff for job-graph-failures

Version: 0.20.8 (the unreleased batch, released together with `job-graph`, `job-graph-cancel` and `job-graph-retry`).
Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The job graph (ADR 0059) now handles failures (ADR 0060, supplementing ADR 0059). No public item, message, field or
state shape is new; the contract's hash is the same, only descriptions in
`docs/reference/contracts/cicada.job-graph.json` changed.

- **Skipped dependents.** In the commit that a job becomes `failed` (or later `skipped` or `cancelled`), every waiting
  job that needs it, directly or through other jobs, becomes `skipped`, without an attempt. Jobs that do not need it
  still run. Before this change they stayed `waiting`.
- **End status.** When no job runs and none can start, the run ends `succeeded` when every job succeeded, else
  `failed` when a job failed, else `cancelled` (reached only once `job-graph-cancel` cancels jobs).
- **The watch on each target.** The `Dispatch` turn that sends a job message also watches the job's target; the run
  unwatches a target once none of its running jobs has it (it keeps its watch on itself). A `Failed` the store wrote
  about the job message of a running job's current attempt (from the run, the job's message type, a `JobStart` of this
  run, job and attempt) ends that job `failed`, reason `the job message failed at the worker (<cause>): <error>`. A
  `Stopped` the store wrote about the target ends every running job dispatched to it `failed`, reason `the worker went
  away (<cause>)`. Other failures of a watched target and notifications a client sent change nothing.
- **Order.** A job message, and the failure of its `Dispatch`, count only while the job is `running`: a client's
  result that ends the job first keeps its status, and the attempt's `dispatchedAt` stays null. Ready jobs start after
  every change of a job's status, so a slot freed by a gone worker starts the next job in that commit.
- **Decoder.** It also refuses a skipped job with an attempt or with no failed, skipped or cancelled need, a waiting job
  with such a need, and an ended run whose status does not follow its jobs.

Proved by `crates/cicada-actors/tests/job_graph.rs` (27 tests on virtual time; new:
`a_failed_job_skips_its_dependents_transitively_and_only_them`,
`a_job_message_that_fails_at_the_worker_fails_its_job_and_skips_its_dependents`,
`a_deleted_worker_fails_every_job_dispatched_to_it`, `a_slot_freed_by_a_gone_worker_starts_the_next_job`,
`another_failed_message_of_a_shared_worker_changes_no_job`,
`a_worker_that_reports_and_stops_in_one_turn_has_its_result_taken`, `notifications_a_client_sent_change_no_job`,
`a_failed_dispatch_of_a_job_that_got_its_result_first_changes_nothing`,
`the_run_unwatches_a_target_once_no_running_job_has_it`), the unit tests in `crates/cicada-actors/src/job_graph.rs`
(skip loop, end status, decoder checks) and `tests/wire_formats.rs` (an ended state with skipped jobs, and the state a
run writes after a failed job).

## 1. Failures in the job graph section of the actors guide

**Where.** `docs/guide/actors.md`, `## Job graph` (the section the `job-graph` handoff asks for). Replace its bullet
"A failed job blocks the jobs that need it (they stay `waiting`)…" and its caveat "A worker that fails or goes away
after it took its job message leaves its job `running`…".
**Text.**
- A failed job skips every waiting job that needs it, directly or through others: they become `skipped`, without an
  attempt, in the same commit. Jobs that do not need it still run. The run ends when no job runs and none can start:
  `succeeded` when every job succeeded, else `failed` when a job failed, else `cancelled`.
- The run watches each job's target from the turn that sends the job message, and unwatches it when none of its
  running jobs has it. A job ends `failed` when its job message fails at the worker (the handler failed until its
  retries ran out, or refused it), with the reason `the job message failed at the worker (<cause>): <error>`, and when
  its worker goes away (stopped itself, deleted, or removed with its parent), with `the worker went away (<cause>)`.
  Then its dependents are skipped and the next ready job starts.
- A shared worker's other failures reach the run while it watches that worker; each is one ignored turn.
**Caveats.**
- What the run cannot see: work the worker handed on to another actor; a worker that took the job message and then
  lost its outside work without failing a message, stopping or reporting; the failure of a later message of the worker.
  Such a job stays `running`.
- A target must stay until its job reports: a target that hands work on and then stops itself fails its job.
- A `Stopped` does not say which instance went away: a target deleted and created again before the run handles the
  `Stopped` fails a job dispatched to the new one (rare).
- A worker that reports and stops in one turn normally has its result taken (the commit writes its sends before its
  notifications). But when the run's turn on that result is rescheduled (cause `node` or `timeout`), the `Stopped`
  overtakes it: the job ends `failed` and the result is refused. `job-graph-retry` can run the job again.
- A client's result may end an attempt before its `Dispatch` ran; the job message is then never sent and the
  attempt's `dispatchedAt` stays null.

## Blocking

None.

## Status

Ran: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo test
--workspace` with PostgreSQL (3,285 passed, 0 failed), `cargo test -p cicada-examples --test docs` (11 passed) and
`--test skill` (18 passed), `openspec validate job-graph-failures --strict` (valid); the gate's result is in the review
request. All text above is proposed; no protected prose is written. The guide section itself is still to be written
from the `job-graph` handoff.
