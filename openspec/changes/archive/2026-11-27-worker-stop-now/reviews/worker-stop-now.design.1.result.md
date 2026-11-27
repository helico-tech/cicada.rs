Verdict: not yet

## Blocking

1. **Stop Now can be lost between the tray's disconnect and the worker's stop.** Where: design.md D1 and D3, the menu-bar-tray spec. `Tray::disconnect` sets `Stopping` at once. The worker only enters `Stopping` later, after the oneshot, `run_with`'s shutdown future, `Stop` and the select in `run_until_stopped` (node.rs). A `stop_now` in that gap reaches `end_grace` before the stop has begun. D1 says such a call "does nothing and is not remembered", yet the tray has already moved `until` to now plus the cancel timeout. The worker then drains for the full grace period, and the countdown is wrong. The design's own example (`disconnect(); stop_now();` with no yield) hits this gap, and so will the tray test. Fix: decide where the request is kept. My pick: `StopNow` in cicada-app sets a flag that `run_until_stopped` applies right after `worker.stop()` enters `Stopping`, or calls `end_grace` itself once the stop has begun. Add a scenario: disconnect, then stop_now with no yield, ends within the cancel timeout.

## Notes

1. `until` should become min(old until, now + cancel timeout). As written, a second Stop Now, or one picked during the cancel phase, pushes the countdown later than the real end.
2. Say that `end_grace` and `StopNow::now` are sync (they are called from the AppKit main thread) and say what `StopNow` holds. A `Worker` clone keeps the source and its observers alive for as long as a host keeps `WorkerReady`.
3. "No further break" is not true against 0.25.0: losing `PartialEq`/`Eq` and the new field must go into the 0.26.0 BREAKING line. The node.rs test `worker_ready_runs_once_…` uses `assert_eq!` on `WorkerReady` and must change.
4. The remote-workers requirement "Worker shutdown preserves…" still says "drain for the grace period". Make it a MODIFIED requirement, or point to it. The handoff should also name docs/architecture.md (~line 376, the Worker stop).
5. If `end_grace` lands while the stop still waits for the poll loop, the stop can take up to twice the cancel timeout. This is unlikely, but the spec says "within".

## Owner questions

None.

## Not checked

The AppKit code and the cicada-app `Stop` internals beyond node.rs. No guarantee row or wire format changes, so no ADR is needed.
