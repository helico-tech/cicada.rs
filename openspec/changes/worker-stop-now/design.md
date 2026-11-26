# Design

The menu should offer Stop Now while a worker drains, to end its running turns at once instead of waiting out the grace
period. Aborting the tray's task does not do it: each turn and its lease renewal run in tasks of their own (design review
of `tray-menu-bar`). The worker already cancels its turns when the grace period of a stop ends; Stop Now ends that grace
period early.

```rust
tray.disconnect();   // Stopping { until: stop + grace + cancel timeout }
tray.stop_now();     // the grace period ends now: running turns are cancelled and give their leases back
```

## Decisions

1. **`Worker::end_grace()` in the core.** It ends the grace period of a stop that has begun: `stop_with_grace` waits for
   the running turns, the grace period, or this signal, whichever comes first, and then goes on as at the end of the
   grace period (cancel the turns, wait at most the cancel timeout, leave behind those that ignore it). Called before a
   stop begins it does nothing and is not remembered, so it never cancels a later stop's grace period; called during a
   stop, also before the stop reaches its wait, it takes effect (a `CancellationToken` set only while `Stopping`).
   Not taken: cancelling the turns' token directly (it would bypass the cancel-timeout bound and the stop's
   bookkeeping).
2. **A handle in `WorkerReady`.** `WorkerReady` gains `stop_now: StopNow`, a new public `Clone + Debug` handle whose
   `now()` calls the worker's `end_grace()`; it outlives the worker harmlessly (a call after the stop does nothing).
   `WorkerReady` drops `PartialEq` and `Eq` (a handle has no meaningful equality); it changed already in the unreleased
   0.26.0, so no further break reaches a release. Only a worker over HTTP reports `WorkerReady`; `serve` and a worker
   on the store keep their signal-driven stop. Not taken: a second shutdown future on `run_with` (a signature change
   for every caller).
3. **`Tray::stop_now()`.** While `Stopping` with a ready worker, it calls the kept `StopNow` and sets `until` to now plus
   the cancel timeout, so the menu's countdown follows; the state stays `Stopping` until `run_with` returns and then
   becomes `Offline` as for any stop. In any other state, or while stopping a worker that was never ready, it does
   nothing.
4. **The menu item.** The menu model gains a Stop Now row while `Stopping` with a ready worker, after the header; the
   AppKit view renders it as a plain item whose action calls `stop_now`. The countdown in the header stays.

## Tests

Core, on virtual time: a turn that runs past a stop's start is cancelled at once by `end_grace` and the stop returns
within the cancel timeout, not the grace period; `end_grace` before a stop does nothing and the later stop keeps its full
grace period; `end_grace` twice, or after the stop, does nothing. App: `WorkerReady::stop_now` ends a worker over HTTP's
drain early. Tray: `stop_now` while stopping a ready worker ends `Offline` well before the grace period, its running
turn listed as abandoned, and `until` moves to now plus the cancel timeout; `stop_now` while online, offline or failed
does nothing. Menu model: the Stop Now row only while stopping a ready worker.

## Docs

The handoff gives the app guide `WorkerReady::stop_now`, the tray section Stop Now, and the runtime guide (core)
`Worker::end_grace` next to `stop_with_grace`; the changelog's 0.26.0 section names them.
