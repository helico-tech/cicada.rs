# Design

A host that runs work next to its node (Factory's runner and pool member) spawns it from `on_start` today. When that
work panics or ends, the node runs on and `/ready` stays 200 (Factory issue 0010). `App::task` ties a long-running
task to the node's life: started with the node, told to stop at shutdown and waited for, and an early end stops the
node with an error, so a supervisor restarts the process (the owner's choice, 2026-11-25).

```rust
App::new("factory")
    .host(runner_type)
    .task("runner", |system, stopping| async move {
        loop {
            tokio::select! {
                () = stopping.wait() => return Ok(()),     // shutdown: end in good order
                job = next_job() => run(&system, job?).await?,
            }
        }
    })
    .run()
```

## Decisions

1. **`App::task(name, f)`.** `f: FnOnce(ActorSystem, Stopping) -> Fut`, `Fut: Future<Output = anyhow::Result<()>>`,
   both `Send + 'static`. Several calls give several tasks; the name appears in logs and errors. `Stopping` is a small
   `Clone` type of `cicada-app` with `async fn wait(&self)` and `fn is_set(&self) -> bool`. Not taken: tokio-util's
   `CancellationToken` in the public API (its next major version would break hosts); an `on_start` that returns a
   handle (a host still has to watch it).
2. **Started after `on_start`, before `serve` listens.** In `serve` and `worker`, the tasks start once every
   `on_start` function has finished, in the order of the calls, so a task finds what `on_start` created and `/ready`
   answers only once the tasks run. A worker over HTTP has no `ActorSystem`, so an `App` with a task refuses to run
   one (exit code 2), as with `on_start`.
3. **An early end stops the node with an error.** A task that returns `Ok`, returns `Err` or panics before the stop
   was asked for starts the stop exactly as a signal does (decision 4: `Stopping` and the node's stop together, the
   store last), and the command fails with exit code 1 and the error
   `task <name> ended before the stop: <error or panic text>`. An `Ok` counts too: a task is meant to run until the
   stop. Not taken: `/ready` turning 503 while the node runs on (a half-working node that nothing restarts); a choice
   per task (no host needs both).
4. **Shutdown: tasks and node together, in one grace period.** On SIGTERM or SIGINT, `Stopping` is set and the node's
   stop begins at the same moment: the node claims no more and `/ready` answers 503 at once, as today, and running
   turns get the node's `shutdown_grace_period`. The tasks are waited for within that same period; a task still
   running at its end is aborted with a warning naming it and not waited for any longer (a task that never yields
   cannot be aborted, and must not hang the stop). A task can still send through the `ActorSystem` while it
   ends (a stopping node refuses only to claim), and the store closes after the tasks and the node have stopped. So the
   worst case of a stop stays what `docs/operations.md` counts today (grace + cancel timeout + drain). A task that
   returns `Err` or panics after the stop was asked for is logged as a warning and does not change the exit code: the
   stop was in good order. Not taken: tasks first, then the node (the node would claim and stay ready for a whole
   extra grace period: design review, blocking 1).
5. **Every way out stops the tasks.** Once the tasks run, every path that ends the command stops them the same way
   before the store closes: a stop, an early end of a task, a failed write of `Listening on`, a failed HTTP server. So
   no task outlives the command, also under `run_with` on the host's runtime (design review, blocking 2). When an early
   end and a signal come together, the early end recorded first wins, as for the worker over HTTP today.
6. **Names are unique.** Two tasks with one name refuse `serve` and `worker` before they start (exit code 2, "two tasks
   named …"); the operator commands run no task and are not refused. The refusal of a worker over HTTP names both
   `on_start` and tasks and comes, as today, before the config file is read; the `worker` help (`WORKER_HELP`) names
   both, so `docs/reference/cli.md` is regenerated (`CICADA_UPDATE_CLI=true`).
7. **An ADR.** A rule hosts build on (an early end stops the node, the order at shutdown): ADR 0064, "Background tasks
   of an app".

## Tests

In `crates/cicada-app` (on SQLite in memory, with the command's `Stop` driven by the test):
- a task that returns `Ok` early, one that returns `Err` and one that panics: the node stops, the command fails with
  exit code 1 and the error names the task; the other task saw `Stopping`;
- a stop: each task sees `Stopping`, `/ready` answers 503 while they end, and the command ends with exit code 0; a task
  that ignores it is aborted at the end of the grace period with the warning; a task that returns `Err` after the stop
  gives a warning and exit code 0;
- a failed write of `Listening on` stops the tasks;
- the task starts after `on_start`, and before `serve` writes `Listening on`;
- two tasks with one name, and an `App` with a task as a worker over HTTP, are refused with exit code 2.
No E2E test is added; the finishing gate runs with E2E, as the change touches `serve` and `worker`.

## Docs

Nothing breaks. The handoff gives the app guide `App::task`, `Stopping`, the early-end rule and the shutdown (its
Lifecycle paragraph), tells `docs/operations.md` that a stop's worst case is unchanged and that tasks share the grace
period, names `docs/architecture.md` where it says `on_start` is refused over HTTP (tasks too now), and suggests moving
work out of `on_start` into a task. `docs/reference/cli.md` changes only through the regenerated `worker` help.
