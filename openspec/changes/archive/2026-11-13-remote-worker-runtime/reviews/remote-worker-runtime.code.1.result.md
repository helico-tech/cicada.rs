Verdict: fix needed

Blocking findings:

1. `crates/cicada/src/runtime/worker.rs:536`: `run_due_turns` checks state before awaiting initialization, but never again before leasing. Block registration during a manual call, stop the worker, then unblock registration: the stopped worker sends a new lease request, only rejecting at handoff. This can claim messages after shutdown and leave leases behind when cleanup fails. Recheck/admit dispatch under the lifecycle synchronization after initialization, before issuing the request; add the manual counterpart of the existing stop-during-start test.

2. `crates/cicada/src/runtime/executor.rs:454`: shutdown aborts the keeper, then the operator-cancelled branch calls `finish`. A pending completion races only `lost_signal` and its full-TTL budget (879), so with the watchdog gone it can survive the confirmed lease deadline. Reproduce with operator cancellation winning, shutdown while work is still stopping, and a hanging `complete_turn`. Keep the source lease keeper/watchdog active through Cancelled completion, or otherwise enforce its current deadline independently; test expiration during this shutdown path. Preserve local-node behavior.

3. `openspec/changes/remote-worker-runtime/specs/remote-workers/spec.md:132`: the “Handler time” scenario lacks a Worker test for `current_time`. The parity actor records `now`, but none of the Worker tests calls `current_time`; the existing local observer test does not exercise worker request transit. Add a paused-time handler test with delayed lease delivery, asserting frozen `now` and elapsed time measured from the worker request.

Notes: The worker-only strict guard and direct submillisecond timing checks match the documented departures. Version, documentation and generated-reference changes are consistent.

Not checked: No tests, builds or E2E were run; relied on the builder's reported checks. Findings are from static review of the requested head and diff.
