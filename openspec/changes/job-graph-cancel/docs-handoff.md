# Docs handoff for job-graph-cancel

Version: 0.20.8 (the unreleased batch, released together with `job-graph`, `job-graph-failures` and `job-graph-retry`).
Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The job graph (ADR 0059) can cancel one job or the whole run, and tell a running worker (ADR 0060, supplementing ADR
0059). The run's contract changed (`docs/reference/contracts/cicada.job-graph.json`: the messages `Cancel` and
`SendCancel`, the job field `cancelType`, the attempt outcome `cancelled`); the end report `JobGraphEnd` keeps its
shape (only the descriptions of the statuses `cancelled`, formerly "not set by this release", changed).

- **`Cancel { job }`** (public message of `cicada.job-graph/<run>`): one job, or every job when `job` is absent or
  null. A waiting job becomes `cancelled` at once; a running job too, its attempt ending with outcome `cancelled` and
  reason `cancelled`, and its target is unwatched unless another running job has it. Ended and skipped jobs, and an
  ended run, are left alone: a repeated `Cancel` changes nothing. An unknown job name is refused for good, the state
  unchanged. In the same commit the cancelled job's dependents are skipped, ready jobs start in the freed slots, and
  the run may end: `cancelled` when no job failed, else `failed`.
- **The cancel message type**: `Job::with_cancel_type("CancelBuild")` (field `cancelType`, 1–200 characters, no NUL,
  checked by `JobGraphState::new` and the decoder). When a job whose job message was sent is cancelled, the run sends
  itself an internal `SendCancel` in the `Cancel` commit; in a later turn that sends `CancelBuild` with a `JobCancel
  { run, job, attempt }` (new public type) to the job's target, with the shared retry policy and the `Cancel`'s
  correlation id. A gone worker fails only that `SendCancel`, never the `Cancel`. A `SendCancel` a client sends does
  nothing.
- **Not told**: a job without a cancel type (its outside work goes on; its late result is refused), a job not yet
  dispatched (its `Dispatch` then sends nothing), and work the worker handed on (the worker passes the cancel along).
- **`AttemptOutcome`** (new public type): `JobAttempt.outcome` is now `succeeded | failed | cancelled`; a worker's
  `Result` keeps `JobOutcome` (`succeeded | failed`), so a worker cannot report `cancelled`.
- **Decoder**: also refuses a cancelled job whose last attempt is open or ended other than `cancelled`.

Proved by `crates/cicada-actors/tests/job_graph.rs` (39 tests on virtual time; new:
`cancel_of_one_running_job_skips_its_dependent_and_refuses_its_late_result`,
`cancel_of_the_whole_run_cancels_every_waiting_and_running_job_and_reports`,
`cancel_of_the_whole_run_after_a_failure_ends_it_failed`,
`cancel_of_one_job_of_a_run_that_otherwise_succeeds_ends_it_cancelled`,
`a_cancel_frees_a_slot_for_the_next_job_in_its_commit`,
`cancel_of_an_ended_or_skipped_job_or_an_ended_run_changes_nothing`, `cancel_of_an_unknown_job_is_refused_for_good`,
`a_failed_dispatch_reported_after_the_cancel_leaves_the_job_cancelled`,
`a_worker_is_told_through_the_jobs_cancel_type_in_a_later_turn`, `a_gone_workers_send_cancel_fails_alone`,
`a_job_cancelled_before_its_dispatch_is_never_sent_anything`, `a_send_cancel_sent_by_a_client_sends_nothing`), the unit
tests in `crates/cicada-actors/src/job_graph.rs` (`a_cancel_type_out_of_bounds_is_refused_and_names_the_job`,
`inconsistent_cancelled_jobs_are_refused`) and `tests/wire_formats.rs` (samples of `Cancel`, `SendCancel`, a
`CancelBuild(JobCancel)` family, a cancelled state, and the state a cancelled run writes).

## 1. Cancel in the job graph section of the actors guide

**Where.** `docs/guide/actors.md`, `## Job graph` (the section the `job-graph` and `job-graph-failures` handoffs ask
for): a part on cancelling, after the failures.
**Text.**
- Send the run `JobGraphMsg::Cancel { job: Some("build".into()) }` to cancel one job, or `Cancel { job: None }` to
  cancel the whole run. Waiting and running jobs become `cancelled` in that commit; a cancelled job's dependents are
  skipped, the next ready job starts in the freed slot, and the run ends `cancelled` when no job failed (`failed` when
  one did). Jobs that ended or were skipped stay as they are, so cancelling twice is harmless; an unknown job name is
  refused for good.
- To tell a running worker, give the job a cancel message type: `Job::new("build", &builder, "Build",
  input)?.with_cancel_type("CancelBuild")`, and let the worker's family declare it: `enum BuilderMsg {
  Build(JobStart), CancelBuild(JobCancel) }`. The worker gets `JobCancel { run, job, attempt }` in a turn of the run
  after the `Cancel`, and stops the work of that attempt. A Rust block can follow `BuilderMsg` and `BuilderActor` in
  `crates/cicada-actors/tests/job_graph.rs` and the test
  `a_worker_is_told_through_the_jobs_cancel_type_in_a_later_turn`.
- A worker without a cancel type is not told: its outside work goes on, and its late result is refused. A job
  cancelled before its job message was sent never gets one, nor a cancel message.
- The end report names each job's outcome: `succeeded`, `failed`, `skipped` or `cancelled`; a cancelled job that ran
  has its attempt number and the reason `cancelled`.
**Caveats.**
- A worker can get `JobCancel` before its `JobStart` of the same attempt (when its turn on the job message is
  rescheduled): it then ignores that attempt's job message, e.g. by keeping the cancelled attempts it saw.
- Cancel tells only the job's own target (`to`); a worker that handed work on passes the cancel along itself.
- A gone worker fails only the run's `SendCancel`; the job is `cancelled` from the `Cancel` already.
- A host whose stored runs or messages were made with an older contract of `cicada.job-graph` follows "Changing a
  contract" (`docs/operations.md`, `#changing-a-contract`); the end report's shape is unchanged.

## Blocking

None.

## Status

Ran: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo test
--workspace` with PostgreSQL (3,299 passed, 0 failed), `cargo test -p cicada-examples --test docs` (11 passed) and
`--test skill` (18 passed), `openspec validate job-graph-cancel --strict` (valid); the gate's result is in the review
request. All text above is proposed; no protected prose is written. The guide section itself is still to be written
from the `job-graph` and `job-graph-failures` handoffs.
