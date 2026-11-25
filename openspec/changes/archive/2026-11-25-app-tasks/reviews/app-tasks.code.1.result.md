Verdict: fix needed

## Blocking

1. `crates/cicada-app/src/node.rs:274-276` with `:209-224` (`run_task`, `Tasks::finish`). The early end becomes
   known only when the JoinSet hands back the task's output, but `run_task` sets `Stopping` *before* it returns.
   `finish` wakes on `Stopping` and starts its deadline at once. With `CICADA_SHUTDOWN_GRACE_PERIOD=0s` (allowed;
   `worker_http.rs:401` uses it) on the multi-thread runtime `App::run` builds, the deadline can pass before the
   runner's task completes. That happens when the runner's thread is preempted between `stopping.set()` and the end of
   the task (about 1 ms, the timer's tick). Then the timeout branch logs "Task runner did not end within the shutdown
   grace period of 0ns; aborted", `early_end` stays `None`, and the command exits 0. The spec says an early end
   SHALL exit 1. How to see it: reason through the code, or put a `std::thread::sleep(5ms)` between `stopping.set()`
   and the return in a scratch copy and run the early-end test with grace 0s. Fixed: the exit code does not depend on
   timing. For example, `run_task` records its early end in a slot that `Tasks` shares (first writer wins) *before*
   it sets `Stopping`, and `finish` reads that slot. Add a test: an early end with a grace period of 0s still exits 1.

## Notes

1. Scenario "The server fails after the tasks started" has a test only for the failed write of `Listening on`, not
   for a failed HTTP server. That path is hard to trigger, and the code joins it the same way, so I do not block on it.
2. The worker scenario "A stop" says "the node claims no more", but
   `a_stop_of_a_worker_ends_its_task_with_exit_code_0` does not check it (it holds by the node's existing stop).
3. `crates/cicada-app/src/lib.rs:33` is 135 columns, and node.rs adds a few lines of 121-122 columns. rustfmt does
   not wrap comments, but the house style is 120.
4. When the deadline passes, the warning names every task not yet joined. A task that already ended but is not yet
   joined is named as "aborted" too (the same race as finding 1).

## Not checked

I did not run the test suite or the gate. I relied on the builder's report and the gate evidence for 75fabb37. I did
not reproduce finding 1 by running it; it comes from reading the code. I did not run the E2E tests.
