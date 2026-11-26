# Docs handoff for tray-worker-control

Version: 0.25.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`cicada-tray` adds `Tray`, a portable model that runs an HTTP worker in the host process. It exposes the current
`TrayState` and a snapshot of active turns and the last 50 finished turns. `App::type_names()` lists types in the
order they were hosted. `App::on_worker_ready` calls the host when the HTTP worker has started and gives it
`WorkerReady { grace_period, cancel_timeout }`.

The virtual-time tests in `crates/cicada-tray/tests/control.rs` drive the worker through the actor API with
`Tray::with_env` pointing to a test context file. They cover connecting while idle, reconnecting with a fresh
`App`, rejecting bad contexts and incompatible apps, retrying an unreachable server, draining a running turn,
naming an abandoned turn and evicting the oldest finished turns. The `cicada-app` tests cover type order and the
readiness hook after a server retry and contract registration.

## 1. A menu bar worker

**Where.** `docs/guide/app.md`, add `A menu bar worker` after `A worker over HTTP`.

**Text.** Build a `cicada_tray::Tray` with `Tray::new(|| App::new().host(MyActor))`. The tray calls the factory
again on every connection it starts. Call `connect("production")` on a Tokio runtime to run an HTTP worker against
the named operator context. The worker reads the process environment by default. Use `with_env` to pass an
explicit map, for example one setting `CICADA_CONFIG` to a chosen config file.

Subscribe with `state()` and read the current value. The worker moves from `Offline` through `Connecting` to
`Online`, even if it has no work. If the worker command fails, `Failed` carries its error. Call `disconnect()`
to stop claiming and give running turns their grace period. `Stopping` exposes the stop time plus the settled
grace period and cancel timeout. A successful stop returns to `Offline`. Use `App::type_names()` to list the
factory's hosted types in order. Other hosts embedding an HTTP worker can subscribe to startup with
`App::on_worker_ready`, which runs after the server check and contract registration.

`turns()` gives the running turns and the last 50 finished turns for this tray. Watch `turn_changes()` for a
redraw. A running turn shows its actor, message type, attempt and claim time. A finished turn adds the end time
and the outcome. `Abandoned` means the worker saw no commit succeed, for example when shutdown cancelled its
handler.

**Caveats.** The worker follows the retry and shutdown rules of the HTTP worker command. An `App` using
`on_start` or `task` is refused because those callbacks require a local `ActorSystem`. Turn history lives in
memory. The macOS menu comes in a later change.

## Blocking

None.

## Status

The app and tray tests passed, along with 11 docs tests and 51 guide doctests. The guide text is proposed for the
docs session.
