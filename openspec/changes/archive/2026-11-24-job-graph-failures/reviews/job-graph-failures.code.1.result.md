Verdict: pass

## Blocking

None.

Checked against design decisions 1–8 and the spec delta (`ffc0935..5c02109`):

- Tasks 1–6 are ticked and in the diff, including the `JobAttempt` doc comment on a null `dispatchedAt`.
- Every scenario has a test: the virtual-time tests (27), the unit tests (skip loop, end status, decoder) and the wire-format samples. The older scenarios keep their `job-graph` tests.
- `settle` skips, starts, ends, then unwatches, in one commit. `dispatch` and `undispatched` require `running`. The watch is set in the send's commit. `job_message_failed` checks `from`, run, job, current attempt, target and message type. `worker_gone` fails only dispatched jobs. A target is unwatched only when no running job has it, never the run itself. An unwatch of an unheld watch is ignored (`turn.rs:679`).
- The contract hash is unchanged; only the reference's descriptions are regenerated. The version is 0.20.8. No protected docs are edited. ADR 0060 and its README row exist. The CHANGELOG and the handoff (27 tests) match the code.

## Notes

1. The decoder refuses a skipped job with no failed, skipped or cancelled need. That is stricter than the spec wording ("whose needs all succeeded"), but it matches decision 8.
2. No test sends a run-sent `Failed` for a stale attempt or another job on a shared worker. The checks are in the code; add the test in `job-graph-retry`, where they first matter.
3. `job_message_failed` does not check `dispatchedAt`. Harmless: an undispatched attempt has no job message.

## Not checked

No tests or builds run; I relied on the recorded gate pass. Not checked: E2E, and the documented races (a rescheduled `Result` turn, a target recreated before its `Stopped`) beyond reading the code.
