# Design

The menu should offer Stop Now while a worker drains, to end its running turns at once instead of waiting out the grace
period. Aborting the tray's task does not do it: each turn and its lease renewal run in tasks of their own (design
review of `tray-menu-bar`). The worker already cancels its turns when the grace period of a stop ends; Stop Now ends
that grace period early.

```rust
tray.disconnect();   // Stopping { until: stop + grace + cancel timeout }
tray.stop_now();     // the grace period ends now: running turns are cancelled and give their leases back
```

## Decisions

1. **`Worker::end_grace()` in the core.** It ends the grace period of a stop that has begun: `stop_with_grace` waits for
   the running turns, the grace period, or this signal, whichever comes first, and then goes on as at the end of the
   grace period (cancel the turns, wait at most the cancel timeout, leave behind those that ignore it). Called before a
   stop begins it does nothing and is not remembered, so it never cancels a later stop's grace period; called during a
   stop, also before the stop reaches its wait, it takes effect (a `CancellationToken` set only while `Stopping`,
   checked under the lock that sets `Stopping`). It also ends the stop's wait for the polling loop, which is then
   aborted, so the stop returns within one cancel timeout of the call. It is a plain call, not `async`. Manual
   batches are cancelled too (their tokens are children of the turns' token). Not taken: cancelling the turns' token
   directly (it would bypass the cancel-timeout bound and the stop's bookkeeping).
2. **A handle in `WorkerReady` that remembers.** `WorkerReady` gains `stop_now: StopNow`, a new public
   `Clone + Debug` handle that holds only a `CancellationToken`, not the worker; `now()` is a plain call that sets it.
   The worker over HTTP in `cicada-app` watches that token next to its stop: once the stop has begun (`worker.stop()`
   started) and the token is set, before or after, it calls `worker.end_grace()`. It must not poll the token branch
   before `worker.stop()` has set `Stopping` (as `join!` could), or the call is lost. So a Stop Now that comes between
   the host's stop signal and the worker's own `Stopping` is kept, not lost (design review). Set before the stop signal,
   it applies when the stop begins. A call after the stop does nothing. `WorkerReady` drops `PartialEq` and `Eq` (a
   handle has no meaningful equality); against 0.25.0 that, the new field and the earlier `types` change are all in the
   0.26.0 **BREAKING** line; the `node.rs` test that compares whole `WorkerReady` values compares fields instead. Only a
   worker over HTTP reports `WorkerReady`; `serve` and a worker on the store keep their signal-driven stop. Not taken: a
   second shutdown future on `run_with` (a signature change for every caller).
3. **`Tray::stop_now()`.** While `Stopping` with a ready worker, it calls the kept `StopNow` and sets `until` to the
   earlier of the current `until` and now plus the cancel timeout, so the menu's countdown follows and a second Stop
   Now never moves it later; the state stays `Stopping` until `run_with` returns and then
   becomes `Offline` as for any stop. In any other state, or while stopping a worker that was never ready, it does
   nothing.
4. **The menu item.** The menu model gains a Stop Now row while `Stopping` with a ready worker, after the header; the
   AppKit view renders it as a plain item whose action calls `stop_now`. The countdown in the header stays.

## Tests

Core, on virtual time: a turn that runs past a stop's start is cancelled at once by `end_grace` and the stop returns
within the cancel timeout, not the grace period, also when the call comes while the stop waits for the polling loop;
`end_grace` before a stop does nothing and the later stop keeps its full grace period; `end_grace` twice, or after the
stop, does nothing. App: `WorkerReady::stop_now` ends a worker over HTTP's drain early. Tray: `stop_now` while stopping
a ready worker ends `Offline` well before the grace period, its running turn listed as abandoned, and `until` moves to
now plus the cancel timeout; `stop_now` while online, offline or failed does nothing; `disconnect` then `stop_now` at
once (before the worker's own `Stopping`) still ends within the cancel timeout; a second `stop_now` keeps the earlier
`until`. (`stop_now` never calls `Connection::stopping`, which recomputes it); a `StopNow::now` set before the stop
signal ends the grace period as soon as the stop begins. Menu model: the Stop Now row only while stopping a ready
worker.

## Docs

The handoff gives the app guide `WorkerReady::stop_now`, the tray section Stop Now, and the runtime guide (core)
`Worker::end_grace` next to `stop_with_grace`, and names the worker stop section of `docs/architecture.md`; the
changelog's 0.26.0 section names them, with the `WorkerReady` breaks under its BREAKING line.
