# 0064. Background tasks of an app

Date: 2026-11-25

## Status

Accepted for app-tasks. Builds on ADR 0027 (the app layer) and ADR 0042 (app workers over HTTP).

## Context

A host that runs work next to its node, such as Factory's runner and pool member, spawns it from `App::on_start`.
Nothing watches it: when it panics or ends, the node runs on without it and `/ready` stays 200, so nothing restarts
the process (Factory issue 0010).

## Decision

- `App::task(name, f)` gives a host a task that lives with the node: `f` gets the `ActorSystem` and a `Stopping`
  signal, and starts after every `on_start` function, in `serve` before it listens and in `worker`.
- A task that ends, fails or panics before a stop stops the node in good order and fails the command (exit code 1,
  the error names the task), so the supervisor of the process restarts it.
- At a stop, `Stopping` is set when the node's stop begins: the node claims no more and `/ready` answers 503 at once,
  and the tasks share the node's shutdown grace period, then are aborted with a warning. The store closes after both,
  so a stop takes no longer than before. An error or panic after the stop is logged and does not change the exit code.
- Every path that ends the command after the tasks started stops them first. Task names are unique.
- A worker over HTTP has no `ActorSystem`, so an `App` with a task refuses to run one.

## Consequences

A host's background work is as visible as its node: a crash of either ends the process. A task must watch `Stopping`
or be aborted after the grace period. A task cannot restart itself through `App`; supervision stays outside the
process. `/ready` keeps its meaning (the node is started and serving).
