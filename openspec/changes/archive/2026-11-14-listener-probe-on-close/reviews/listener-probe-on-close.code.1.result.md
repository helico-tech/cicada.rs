Verdict: pass

Blocking findings: none.

The implementation matches the agreed scoped-future design. The probe borrows the attempt's pool and channel; receive completion drops it before reconnect, and aborting the listener drops it during sleep or pending I/O. Stop still requests abort before awaiting the task, so cancellation of that wait cannot detach a probe. Reconnect timing, readiness transitions and the deferred main-pool close behavior remain unchanged.

The strengthened close regression observes an actual probe notification, captures distinct listener/probe backend PIDs, and checks their eventual disappearance without relying on last-query text. It retains stream closure, unavailable-operation and no-restart assertions. Existing lost-listener, wake-up reconnect and flood tests cover recovery. The builder recorded the regression failing against the unfixed code.

All tasks are ticked; version 0.17.1, changelog and generated OpenAPI version agree. No API, schema or guarantee change was introduced.

Notes: none.

Not checked: tests, clippy, formatting and validation were not rerun; their results are assessed from the builder's report. No independent pending-I/O fault experiment or Docker E2E run was performed.
