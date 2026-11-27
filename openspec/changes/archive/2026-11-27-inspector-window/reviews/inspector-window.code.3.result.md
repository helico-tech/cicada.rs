Verdict: fix needed

1. `crates/cicada-inspector/src/tests.rs:305`. The deadline test can hang the gate. If the refresh skips a request such as `/nodes`, the event receiver waits forever while the blocking task at line 278 prevents Tokio's clock from advancing. Use `recv_timeout` in the blocking task and select on its join handle during event collection so the test panics with the missing events when the wall-clock timeout expires.

The deadline test otherwise uses paused time as requested. The switch test awaits the superseded refresh before checking for stale publication. Strict decoding preserves Bearer for an invalid username, omits an invalid password, and strips userinfo. The regression test covers both cases.

Diff whitespace check passed. Cargo tests and the full gate were not run.
