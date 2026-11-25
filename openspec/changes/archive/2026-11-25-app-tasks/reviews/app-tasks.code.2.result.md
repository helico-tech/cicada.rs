Verdict: pass

## Blocking

None. Round 1's finding 1 is fixed at its cause. `run_task` (`crates/cicada-app/src/node.rs:255-285`) now records its
end in the shared `TaskBook` under the lock, and only then sets `Stopping`. `Tasks::finish` (`:212-228`) wakes on
`Stopping` and takes `early_end` from the book, whether the deadline passed or not. So the exit code no longer depends on
when the JoinSet joins the task. `is_set` is checked under the same lock, so "first early end wins" holds too. The new
test `an_early_end_with_a_grace_period_of_zero_still_fails_the_command` (tasks.rs) covers grace 0 on the multi-thread
runtime. The fix also settles round 1's note 4: the abort warning names only `book.running`, the tasks that have not
recorded an end. The fix breaks nothing I can see. `stop_all`, `serve_and_close` and `worker` call `finish` as before.
The moved warnings test (`tests/task_logs.rs`) is unchanged apart from `Flag` becoming `AtomicBool`. The CHANGELOG and
the handoff name it. The comment lines are wrapped; the four lines over 120 columns left in node.rs come from older
commits. Gate evidence for f32ca855: full pass and e2e pass.

## Notes

1. `run_task` formats the user's error (`format!("{e:#}")`) and calls `tracing::warn!` while it holds the std mutex.
   If a user's `Display` panics there, the mutex is poisoned and `finish`'s `expect` panics the command. Building the
   warning text under the lock and logging it after the guard is dropped would avoid this. This is an edge case.
2. Round 1's notes 1 and 2 (no test for a failed HTTP server; the worker stop does not check "claims no more") still
   stand. The builder gave its reason for note 1.

## Not checked

I did not run the test suite or the new test. I relied on the gate evidence for f32ca855 and the builder's report.
I reviewed only the round-2 delta (865e000e..f32ca855; the first three commits match round 1's after the rebase, per
`git range-diff`), not the whole change again.
