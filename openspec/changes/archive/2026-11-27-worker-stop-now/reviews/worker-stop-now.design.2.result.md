Verdict: ready for tasks

Round 1 blocking finding: fixed. D2 makes StopNow a CancellationToken holder; the app's worker applies end_grace once the stop has begun, so a Stop Now between disconnect and the worker's Stopping is kept. The scenario "Stop Now right after the stop signal" and the tray test (disconnect then stop_now with no yield) cover it. The 0.26.0 BREAKING line and node.rs test, the min() rule for until, end_grace ending the polling-loop wait, the split requirement and the architecture.md pointer are all handled. Checked against worker.rs, node.rs and tray lib.rs: no new blocking gap.

Blocking: none.

Notes
1. Order in run_until_stopped: the token branch must call end_grace only after worker.stop() has set Stopping. Under `join!` polled token-first, end_grace would run before the state flips and be lost. Say so in the task, or test it (the "right after the stop signal" scenario will catch it).
2. The existing stop path is `stop.wait() => worker.stop().await`. Stop Now must also work when the token is set before the stop signal; D2 says so, but there is no scenario. Add one cheap test.
3. Tray `Connection::stopping(at)` recomputes `until` from `stopping_at`. Today only worker_ready calls it, before any stop_now, so min() is safe. Keep stop_now from calling it again.
4. The remote-workers spec has no scenario for a poll loop that is slow to stop; the Tests section covers it, which is enough.

Not checked: AppKit code, test code that would be written, docs/architecture.md wording.
