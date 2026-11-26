# Design

The first change of the tray line: the model a Mac menu bar shows and drives, with no GUI yet. A library user hands the
tray a function that builds their `App`; the tray connects a worker over HTTP to a named context, disconnects it in good
order, and shows the state of the connection, the registered types and the turns that run.

```rust
let tray = Tray::new(|| App::new("factory").host(runner_type()));
tray.connect("production");                              // worker --context production, in this process
let mut state = tray.state();                            // a watch: Offline, Connecting, Online, Stopping, Failed
let turns = tray.turns();                                // running turns and the last finished ones
tray.disconnect();                                       // stop: claims end, running turns get the grace period
```

## Decisions

1. **A new crate `cicada-tray`, portable, on `cicada-app`.** The model has no GUI and builds on every platform, so its
   tests run in the gate everywhere; the macOS menu (`tray-menu-bar`) comes later behind `cfg(target_os = "macos")`.
   It depends on `cicada-app` and `cicada`, and nothing depends on it (the gate's dependency-tree check lists it as a
   leaf). ADR 0074 records the crate and its place next to `cicada-app`.
2. **A factory, a fresh `App` per connect.** `Tray::new(factory: impl Fn() -> App + Send + Sync + 'static)`.
   `connect(context)` builds an `App`, adds the tray's `TurnObserver` with `App::observe`, and spawns
   `app.run_with(["<name>", "worker", "--context", context], env, sink, stop)` on the host's Tokio runtime; `stop`
   is a future the tray fires. `disconnect()` fires it: the worker stops claiming and drains within its grace period,
   as on SIGTERM. A connect while connected or stopping does nothing. Not taken: a long-lived `App` (`run_with` uses
   it up).
3. **One state, watched.** A `tokio::sync::watch` of `TrayState::{Offline, Connecting { context }, Online { context },
   Stopping { context, until }, Failed { context, error }}`. `Connecting` until the worker reports it is serving (the
   first successful lease call, seen through the observer's first poll, or `run_with` still running after its start
   checks); `Stopping` from `disconnect` until `run_with` returns, with `until` = now + the node's grace period;
   `run_with` returning `Ok` is `Offline`, an `Err` is `Failed` with the error's text. An `App` with `on_start` or a
   task is refused by `run_with` itself (exit code 2 for a worker over HTTP), which the tray shows as `Failed`: no
   second check.
4. **Turns from a `TurnObserver`.** The tray's observer records each running turn (actor, message type, attempt, claim
   time) when it starts and moves it, with its outcome and end time, to a ring of the last 50 finished turns when it
   ends. `turns()` returns a snapshot; a `watch` of a counter tells the menu when to redraw. Nothing is kept on disk.
5. **The registered types.** `cicada-app` gains `App::type_names() -> Vec<&'static str>`, the hosted types in the order
   of `host` calls, so the menu lists them before connecting.
6. **Contexts by name.** The context names the operator contexts of `cicada-app`'s config file (server URL,
   credentials). The tray does not read or write that file here (`tray-settings` does).

## Tests

On virtual time with an in-process server (`cicada-axum` on a `TestActors` store): connect goes `Connecting` then
`Online` and turns appear while a message is handled; disconnect goes `Stopping` then `Offline`, and a turn running at
the stop finishes within the grace period; a bad context or an unreachable server ends `Failed`; an `App` with
`on_start` ends `Failed` with the refusal; a second connect while online does nothing; the ring keeps the last 50
finished turns; `App::type_names` lists the hosted types in order.

## Docs

Nothing breaks. The handoff gives the app guide a short "A menu bar worker" section (the factory, connect and
disconnect, the state) and says the menu itself comes later.
