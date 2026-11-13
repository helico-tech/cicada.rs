Verdict: fix needed

Blocking findings

1. `crates/cicada-actors/tests/debouncer.rs:532–538`: “Redrive into a later burst” is not exercised. The failed burst still has `firstAt = 0`; after advancing to 12 seconds, `submit(...)` runs until idle. Its replacement Flush is capped at 5 seconds, clamped to now, and delivers/clears the value before `Redrive` is sent. The redriven Flush therefore sees empty state. Removing the redrive send and following run leaves all final assertions satisfied. Finish the original burst first without consuming its dead letter, then start a fresh burst whose timer is still in the future. Assert it remains pending and undelivered before redrive; afterward assert early delivery of that new value, clearing, and cancellation of that timer. This supplies the spec scenario and task 2's missing coverage.

Notes

No further blocking findings. The implementation, checked state, failure handling, contract snapshots, version 0.16.3, and documentation otherwise match the written change. The reported deep Failed-notification issue is in unchanged core behavior and remains separate.

Not checked

I read the change artifacts at head, commit log, diff, implementation, tests, example, and relevant architecture/ADRs. I did not run tests, clippy, fmt, or Docker E2E; finishing-check results rely on the builder's report as required by the review brief.
