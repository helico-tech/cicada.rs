Verdict: fix needed

Blocking findings
1. crates/cicada/src/tests/runtime/remote_completion.rs:454 (test at :473): SQLite cancellation coverage uses real-time sleeps to guess when to abort, ignores whether the task already completed, and never proves COMMIT was in flight. All 24 rounds can pass without exercising the required cancellation/settlement boundary. This also contradicts task 3's explicit prohibition on sleep-based runtime race tests and AGENTS.md's virtual-time rule. Replace this timing sweep with synchronized cancellation at known transaction phases (the existing SQLite CommitFault::BeforeCommit/Queued/InFlight hooks provide them), assert the task was actually cancelled, and retry the identical report through the second handle after settlement. Cover both rollback and applied-but-unacknowledged cases deterministically.
2. crates/cicada/src/tests/runtime/remote_completion.rs:225: the test labelled “Server observers and wakes” checks observer silence but never checks dispatch wakes; its systems are unstarted and host no handlers. The scenario's wake requirement therefore has no completion-path test. Add a paused-time test with a running local dispatcher and long poll interval, verifying prompt delivery after both acknowledged and applied-but-unacknowledged remote completion, without server observer events for the remote turn.

Notes
- docs/guide/core.md:820: the example's retry loop only comments on deadlines; consider enforcing a request-start-derived deadline so copied code cannot retry indefinitely during an outage.
- The implementation's validation, fenced commit, explicit Rejected transition, and shared local policy appear consistent with the design. Version and generated-reference changes are scoped correctly.

Not checked
No tests or finishing checks were run, per the review brief; the builder's reported results were trusted. No release, deployment or HTTP-worker behavior was exercised.
