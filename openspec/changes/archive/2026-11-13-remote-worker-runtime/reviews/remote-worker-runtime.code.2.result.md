Verdict: fix needed

Blocking findings:

1. `crates/cicada/src/runtime/executor.rs:456–464`: shutdown now leaves a worker's entire keeper running while awaiting aborted work. If a handler never yields and renewals succeed, that await never finishes, so the keeper renews forever—even after `shutdown_cancel_timeout` elapses and Worker reports Stopped. This contradicts `Worker::stop_with_grace` (worker.rs:449–450), which promises abandoned work's lease expires, and prevents recovery by another worker. Reproduce with a short TTL, a handler blocking beyond several TTLs, a shorter cancel timeout and successful renewals; after stop returns, renewal counts and lease expiry keep advancing. The existing non-yielding test uses a 2-second handler with the default 30-second TTL, so never reaches its first heartbeat. Preserve the watchdog through Cancelled completion, but bound/stop renewal for work abandoned by shutdown; add coverage proving expiry after the cancel timeout while work remains blocked, without premature release. Keep local-node behavior unchanged.

Notes: The original three blockers are addressed: lifecycle admission rejects a manual batch whose registration completes after stop; the keeper now bounds hanging Cancelled completion by lease loss; the paused-time handler test checks frozen now and current_time including request transit. The local-store shutdown path retains its immediate keeper abort.

Not checked: No tests, builds, or E2E were run; relied on the builder's reported checks. Reviewed only previous..head and relevant implementation/specification context.
