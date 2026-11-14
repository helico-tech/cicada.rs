Verdict: ready for tasks

Blocking findings: none.

The scoped probe removes the detached task at its ownership source. The design preserves LISTEN readiness, reconnect delays, probe cadence and the existing close completion boundary. Cancellation after stop requests abort cannot leave a separate Cicada probe task running. The asynchronous cleanup boundary matches sqlx 0.9's PgListener and PoolConnection drop implementations.

The regression observes a real probe notification before closing, captures its notifying backend PID, and checks saved PIDs instead of parameterized query text. This exercises the leak missed by the existing close test. Running it against the unfixed implementation first provides a useful check of the regression itself.

Notes: none. The separately proposed postgres-close-cancellation work covers the main-pool shutdown issue without broadening this fix. Adjacent proposals do not conflict.

Questions for the owner: none.

Checked the change artifacts, architecture, relevant ADRs, current change-feed spec, adjacent proposals, listener/store shutdown code, PostgreSQL tests and local sqlx cleanup implementation. Ran status, openspec list/show and openspec validate listener-probe-on-close --strict; validation passed. Did not implement the design or run Rust/PostgreSQL tests, the workspace suite, or fault experiments.
