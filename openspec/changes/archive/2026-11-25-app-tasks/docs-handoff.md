# Docs handoff for app-tasks

Version: 0.22.0 (additive; the unreleased batch). Refresh the dependency pins of `docs/how-to/install.md` once it is
published.

## What changed

`cicada-app` gets `App::task(name, f)` and the public type `Stopping` (ADR 0064, "Background tasks of an app").

- `f: FnOnce(ActorSystem, Stopping) -> impl Future<Output = anyhow::Result<()>>`, both `Send + 'static`. Several calls
  give several tasks; the name is in logs and errors.
- `Stopping` is `Clone` with `async fn wait(&self)` and `fn is_set(&self) -> bool`. It is set when the node's stop
  begins and never unset.
- Start: in `serve` and `worker` on the store, after every `on_start` function has finished, in the order of the
  calls. Each task is polled once before `serve` writes `Listening on`, so its code before the first `await` has run
  and `/ready` answers only once the tasks run.
- Early end: a task that returns `Ok`, returns `Err` or panics before the stop stops the node in good order (as a
  signal does) and fails the command with exit code 1, whatever the grace period (also 0 s), and
  `task <name> ended before the stop` (`: <error>` or `: <panic message>` added). The other tasks see `Stopping`. An
  early end recorded before a signal wins.
- Stop: on SIGTERM/SIGINT (or an early end, a failed write of `Listening on`, a failed HTTP server), `Stopping` is set
  when the node's stop begins: the node claims no more and `/ready` answers 503 at once. The tasks share the node's
  `shutdown_grace_period`; a task still running at its end is aborted with the warning
  `Task <name> did not end within the shutdown grace period of <grace>; aborted` and not waited for. `serve` keeps
  answering HTTP (with `/ready` at 503) until the node and the tasks have stopped, then drains HTTP (up to 5 s) and
  closes the store. The store closes after both, so a task can still use its `ActorSystem` while it ends.
- An `Err` or panic after the stop is a warning (`Task <name> failed after the stop: …`) and leaves exit code 0.
- Two tasks with one name refuse `serve` and `worker` with exit code 2 (`two tasks named <name>`); operator commands
  run no task and are not refused.
- A worker over HTTP refuses an `App` with a task, as with `on_start` (exit code 2, before the config file or a
  request). The message names `App::on_start or App::task`; the `worker` help says "A binary with App::on_start or
  App::task cannot run over HTTP." (`docs/reference/cli.md` regenerated).

Tests: `crates/cicada-app/tests/tasks.rs` (start order, early end `Ok`/`Err`/panic in `serve` and `worker`, an early
end with a grace period of 0 s, `/ready` 503 while a task ends, a worker's stop, a failed `Listening on`, duplicate
names), `crates/cicada-app/tests/task_logs.rs` (abort after the grace period and an error after the stop) and
`on_start_and_a_task_are_refused_over_http_before_any_request` in
`crates/cicada-app/tests/worker_http.rs`. Rustdoc example on `App::task` (compiled, `no_run`).

## 1. App guide: background tasks

**Where.** `docs/guide/app.md`, `Run it as a binary`, after the paragraph `**Host hooks.**` (a new paragraph
`**Background tasks.**`).
**Text.** Work that runs next to the node, such as a loop that takes jobs from outside, belongs in `App::task`, not in
a task spawned from `on_start`: nothing watches the latter, so a crash leaves the node running with `/ready` at 200.
`App::task("runner", |system, stopping| async move { … })` starts after every `on_start` and before `serve` listens.
The task must run until the stop: if it returns, fails or panics first, the node stops and the command exits with code
1 naming the task, so the process supervisor restarts it. Watch `stopping.wait()` (for example in a `tokio::select!`)
and return when it completes. Task names are unique. A possible code block (as in the rustdoc of `App::task`):

```rust
App::new()
    .host(RunnerActor)
    .task("runner", |system, stopping| async move {
        loop {
            tokio::select! {
                () = stopping.wait() => return Ok(()),
                job = next_job() => run(&system, job?).await?,
            }
        }
    })
    .run()
```

A doctest block needs its own compiled example; none exists in `examples/` yet (the rustdoc on `App::task` is the
compiled one).
**Caveats.** No restarts inside the process: supervision stays outside. A task that ignores `Stopping` is aborted at
the end of the grace period; one that never yields cannot be aborted and runs on until the process exits.

## 2. App guide: Lifecycle

**Where.** `docs/guide/app.md`, the paragraph `**Lifecycle.**`.
**Text.** Add: at the stop, `Stopping` is set as claims stop; the tasks get the same grace period as the turns, then
are aborted with a warning; serve keeps HTTP answering (with `/ready` at 503) until the node and the tasks have
stopped, then drains HTTP and closes the store. An error of a task after the stop is a warning, not a failure.

## 3. App guide: a worker over HTTP

**Where.** `docs/guide/app.md`, `A worker over HTTP`: "Use the same App without an `on_start` hook" and "`on_start` is
refused before any request".
**Text.** Name tasks too: "without `on_start` hooks or tasks"; "`on_start` and `App::task` are refused before any
request".

## 4. Operations: shutdown

**Where.** `docs/operations.md`, `Shutdown and probes`.
**Text.** The order gains the tasks: stop claims and set `Stopping`, drain/cancel turns while the tasks end within the
same grace period (then aborted), drain HTTP (up to 5 s), close the store. The worst case is unchanged (grace + cancel
timeout + HTTP drain): the platform timeout of 45 s still fits. A task that ends early makes the process exit with
code 1, which a supervisor should restart.

## 5. Architecture

**Where.** `docs/architecture.md`, the app paragraph: "`on_start`, which needs an `ActorSystem`, is refused before any
request".
**Text.** "`on_start` and `App::task`, which need an `ActorSystem`, are refused before any request." A sentence on the
task life may follow: tasks start after `on_start`, an early end stops the node with an error, and at a stop they share
the node's grace period; the store closes after both (ADR 0064).

## Blocking

None.

## Status

After the fix round and the rebase onto main: `cargo test -p cicada-app --all-features` 197 passed, 0 failed;
`cargo test --workspace` with PostgreSQL 3528 passed, 0 failed. The guide, operations and architecture text above is
proposed, not written. `docs/reference/cli.md` was regenerated by `CICADA_UPDATE_CLI=true` and is committed with the
change.
