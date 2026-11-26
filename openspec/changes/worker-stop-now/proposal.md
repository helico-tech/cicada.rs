# Proposal

## Why

The tray's menu should offer Stop Now while a worker drains: end the running turns at once instead of waiting out the
grace period (owner's proposal for `tray-menu-bar`). Aborting the tray's task does not do it: each turn and its lease
renewal run in tasks of their own, so they keep running and may still commit (design review of `tray-menu-bar`,
2026-11-26).

## What Changes

- A way to cut a running stop short: the worker cancels its running turns now, as when the grace period ends, so their
  leases go back to the server. For example `Worker::cancel_turns()`, or a stop whose grace period drops to zero,
  passed through `cicada-app`'s stop to a worker over HTTP.
- `Tray::stop_now()` uses it while `Stopping`, and the menu offers Stop Now with the seconds left.

Decided in the design: `Worker::end_grace()`, a `StopNow` handle in `WorkerReady`, and `Tray::stop_now()`.

## Order

After `tray-menu-bar`, before `tray-settings`.

## Impact

Crates: cicada, cicada-app, cicada-tray.

`crates/cicada` (the worker's stop), `crates/cicada-app` (its stop), `crates/cicada-tray` (`stop_now` and the menu
item). Tests on virtual time. Docs handoff: the app guide.
