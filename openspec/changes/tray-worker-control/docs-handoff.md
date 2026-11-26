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
naming an abandoned turn and evicting the oldest finished turns. They also cover failure when the worker task
panics and reconnecting after Tokio drops it during runtime shutdown. The `cicada-app` tests cover type order and
the readiness hook after a server retry and contract registration.

## 1. A menu bar worker

**Where.** `docs/guide/app.md`, add `A menu bar worker` after `A worker over HTTP`.

**Text.** Build a `cicada_tray::Tray` with `Tray::new(|| App::new().host(MyActor))`. The tray calls the factory
again on every connection it starts. Call `connect("production")` on a Tokio runtime to run an HTTP worker against
the named operator context. The worker reads the process environment by default. Use `with_env` to pass an
explicit map, for example one setting `CICADA_CONFIG` to a chosen config file.

Subscribe with `state()` and read the current value. The worker moves from `Offline` through `Connecting` to
`Online`, even if it has no work. If the worker command fails, `Failed` carries its error. Call `disconnect()`
to stop claiming and give running turns their grace period. A worker task that panics or is cancelled before it
returns also leaves the tray in `Failed`, so a host can connect again on a running Tokio runtime. `Stopping`
exposes the stop time plus the settled grace period and cancel timeout. A successful stop returns to `Offline`.
Use `App::type_names()` to list the factory's hosted types in order. Hosts embedding an HTTP worker can use
`App::on_worker_ready`, which runs after the server check and contract registration.

`turns()` gives the running turns and the last 50 finished turns for this tray. Watch `turn_changes()` for a
redraw. A running turn shows its actor, message type, attempt and claim time. A finished turn adds the end time
and the outcome. `Abandoned` means the worker saw no commit succeed, for example when shutdown cancelled its
handler.

**Caveats.** The worker follows the retry and shutdown rules of the HTTP worker command. An `App` using
`on_start` or `task` is refused because those callbacks require a local `ActorSystem`. Turn history lives in
memory. The macOS menu comes in a later change.

## 2. Crate inventory and skill routes

**Where.** `docs/architecture.md`, `Crates`, after the `cicada-app` row.

**Text.**

```markdown
| `cicada-tray` | A portable model for a menu bar worker ([ADR 0074](adr/0074-a-tray-crate-next-to-the-app.md)). It builds a fresh `App` per connection and runs its HTTP worker in the host process. The model watches connection state, requests graceful shutdown, and tracks running turns and the latest 50 finished turns. |
```

**Where.** `AGENTS.md`, `Map`, after the `crates/cicada-app` row.

**Text.**

```markdown
| `crates/cicada-tray` | The portable tray model (ADR 0074). `src/lib.rs` runs an HTTP worker from an `App` factory and tracks its lifecycle. `src/turns.rs` observes running and finished turns. `tests/control.rs` tests it over HTTP on virtual time. |
```

**Where.** `README.md`, `Layers`, after the `cicada-app` row.

**Text.**

```markdown
| An in-process HTTP worker model for a menu bar | `cicada-tray` | [A menu bar worker](docs/guide/app.md#a-menu-bar-worker) |
```

**Where.** `docs/how-to/install.md`, `Choose the crates`, after the `Use the terminal UI` row.

**Text.**

```markdown
| Run a menu bar worker | Add `cicada-tray` from the same tag for the portable worker model ([app guide](../guide/app.md#a-menu-bar-worker)) |
```

**Where.** `skills/cicada/SKILL.md`, the frontmatter `description` and `4. Routes`.

**Text.** In the description replace `cicada-otel, cicada-containers).` with
`cicada-otel, cicada-containers, cicada-tray).` and `run workers, or deploy` with
`run workers, control an HTTP worker from a tray, or deploy`. Add this row after `Workers and leased turns`.

```markdown
| A menu bar worker: connect by context, drain and observe turns | `docs/guide/app.md#a-menu-bar-worker`, `crates/cicada-tray` |
```

**Caveats.** Add the install row when the pinned release includes `cicada-tray`. Publish the README and skill routes
with the app guide heading. The crate provides the portable model in 0.25.0. The macOS menu comes later.

## Blocking

None.

## Status

The app, tray, docs and guide doctests passed. The app guide, crate rows and skill route are proposed for the docs
session.
