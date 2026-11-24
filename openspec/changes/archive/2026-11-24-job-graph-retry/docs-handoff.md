# Docs handoff for job-graph-retry

Version: 0.20.8 (the unreleased batch, released together with `job-graph`, `job-graph-failures` and
`job-graph-cancel`). Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The job graph (ADR 0059) can retry one failed or cancelled job, and numbers each end of a run (ADR 0061, which
refines ADR 0059's "a repeat for the same run is one" into "the highest end number per run wins"). The run's
contract changed (`docs/reference/contracts/cicada.job-graph.json`: the message `Retry`, the state field `ends`, and
descriptions of `Report` and the job statuses). The end report `JobGraphEnd` gains the required field `end`; it is
declared by the host's family, so it is not in the run's contract file.

- **`Retry { job }`** (public message of `cicada.job-graph/<run>`): the job must be `failed` or `cancelled`, and none of
  its needs `failed`, `skipped` or `cancelled`. In its commit the job becomes `waiting` with its output cleared and its
  attempts kept (still at most 20, the oldest dropped at each start), every `skipped` job becomes `waiting` and the
  skips are computed again (so only the dependents skipped because of this job come back), an ended run goes back to
  `running` with `endedAt` null, and ready jobs start: the retried one with the next attempt number when its needs
  succeeded and a slot is free, else later like any waiting job.
- **Refused for good, state unchanged**: an unknown job or a name out of bounds; a job that is not `failed` or
  `cancelled` (also a second `Retry` of the same job, now `waiting` or `running`); a job with a `failed`, `skipped` or
  `cancelled` need (the reason names the need and says "retry its need first"); a job whose last attempt number is
  `u32::MAX`; an ended run whose `ends` is `u32::MAX`. No cap on the number of retries: a host's own policy caps them.
- **Replaced attempts**: a `Result` for a replaced attempt is refused for good (the store keeps it as a failed message
  of the run); a worker's `Failed` about a replaced attempt's job message, or about another job on the same worker, does
  not touch the retried job; a `SendCancel` of a replaced attempt still sends the `JobCancel` of that attempt, also when
  the `Retry` ran first. A `Stopped` about the target still fails a running retried job dispatched to it.
- **End numbers**: the state's `ends` (`u32`, absent: 0) counts the run's ends; `JobGraphEnd.end` is the number of the
  end it reports. A `Report` sent again repeats the same number; a retried run's next end has a higher one. An end
  reopened by a `Retry` before its `Report` ran is never reported, so numbers can have gaps.
- **Reason in the report**: a job's `reason` is its last attempt's only when that attempt's outcome is the job's
  status; a retried job cancelled before it started again shows `reason: null` (and its old attempt number).
- **Decoder**: a `waiting` or `cancelled` job may have a last attempt that ended `failed` or `cancelled`; an ended run
  needs `ends` of 1 or more.

Proved by `crates/cicada-actors/tests/job_graph.rs` (54 tests on virtual time; new:
`retry_of_a_failed_job_starts_attempt_two_in_its_commit`,
`retry_brings_back_the_dependents_skipped_only_because_of_the_retried_job`,
`a_late_result_of_a_replaced_attempt_is_refused_for_good`,
`a_failed_about_a_replaced_attempt_or_another_job_on_a_shared_worker_leaves_the_retried_job_running`,
`a_retry_before_the_cancel_notice_ran_still_tells_the_worker_of_the_old_attempt`,
`a_retry_after_the_cancel_notice_ran_sends_no_second_cancel`, `a_retried_job_waits_for_a_free_slot`,
`a_retried_job_cancelled_before_it_starts_is_reported_without_the_old_reason`, `a_run_that_ends_twice_reports_end_two`,
`a_report_while_running_again_sends_nothing_and_an_end_reopened_before_its_report_is_skipped`,
`a_decoded_waiting_job_with_old_attempts_starts_with_the_next_attempt`,
`retry_of_a_job_that_is_not_failed_or_cancelled_is_refused_for_good`,
`retry_of_a_job_whose_need_failed_is_refused_and_names_the_need`,
`retry_of_an_unknown_job_or_a_name_out_of_bounds_is_refused_for_good`,
`retry_without_an_attempt_or_end_number_left_is_refused_for_good`), the unit tests in
`crates/cicada-actors/src/job_graph.rs` (`a_waiting_or_cancelled_job_may_keep_old_attempts_that_ended_failed_or_cancelled`,
`an_ended_run_has_ends_of_one_or_more_and_a_running_one_any`,
`reopen_brings_back_only_the_jobs_skipped_because_of_the_retried_one`) and `tests/wire_formats.rs` (samples of
`Retry`, a reopened state with `ends` and old attempts, an end report with `end`, and the states a retried run writes).

## 1. Retry in the job graph section of the actors guide

**Where.** `docs/guide/actors.md`, `## Job graph` (the section the `job-graph`, `job-graph-failures` and
`job-graph-cancel` handoffs ask for): a part on retrying, after cancelling.
**Text.**
- Send the run `JobGraphMsg::Retry { job: "test".into() }` to run a `failed` or `cancelled` job again. It waits again
  and starts with the next attempt number as soon as its needs succeeded and a slot is free, in the same commit when it
  can. The jobs skipped only because of it wait again too; a job that also needs another failed or cancelled job stays
  skipped. Retry that other job as well, needs first: a `Retry` of a job whose need failed, was skipped or cancelled is
  refused for good with a reason that names the need. So is a `Retry` of a job that is not `failed` or `cancelled`, or
  of an unknown job.
- A run that had ended runs again and ends again, and reports that end with a higher `end` number. A report target
  keeps, per run, the report with the highest `end`; one with the same `end` is a repeat. A Rust block can follow
  `CiActor` in `crates/cicada-actors/tests/job_graph.rs` and the test `a_run_that_ends_twice_reports_end_two`.
- The job graph has no retry policy of its own: a host that retries automatically sends `Retry` from its own code (for
  example from its report target, on a failed end report) and keeps its own cap.
**Caveats.**
- Expect gaps in the `end` numbers: an end that a `Retry` reopened before its report went out is never reported. Do
  not rely on reports arriving in order either: compare `end`, not the arrival.
- A worker keys its work and a `JobCancel` by attempt: the cancel of a replaced attempt can arrive after the `Retry`,
  even after the `JobStart` of the new attempt, and must stop only that old attempt's work.
- A late result of a replaced attempt is refused, so a worker always names the attempt of its `JobStart`.
- A retried job cancelled before it started again is reported `cancelled` with no reason; its `attempt` is the old one.
- Any guide block that builds a `JobGraphEnd` needs the `end` field.
- A host whose stored runs or messages were made with an older contract of `cicada.job-graph` follows "Changing a
  contract" (`docs/operations.md`, `#changing-a-contract`); the end report's new required `end` field ships in the same
  release as the job graph, so no released host sees it change.

## Blocking

None.

## Status

Ran: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo test
--workspace` with PostgreSQL (3,317 passed, 0 failed; `cicada-actors --test job_graph` 54 passed), `cargo test -p
cicada-examples --test docs` (11 passed) and `--test skill` (18 passed), `openspec validate job-graph-retry --strict`
(valid); the gate's result is in the review request. All text above is proposed; no protected prose is written. The guide section itself is still to be written from the
`job-graph`, `job-graph-failures` and `job-graph-cancel` handoffs.

## Notes from the code review (added by the change loop)

- A run keeps a job's last 20 attempts. If 20 newer attempts start before a waiting cancel notice of an old attempt
  runs, that attempt is dropped and its cancel is never sent; only many fast retries get there. Worth a caveat.
- The decoder accepts a hand-made running state whose `ends` is already at its maximum; when such a run ends, the last
  result, cancel or failure notice is refused for good and the run stays running. A `Retry` cannot get there.
