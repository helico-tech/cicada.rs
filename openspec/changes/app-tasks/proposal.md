# Proposal

## Why

Factory runs its runner and pool member as tasks it spawns from `on_start`. When one panics or ends, `/ready` stays
200 and the node runs on without running containers (Factory issue 0010, request from the Factory session,
2026-11-25). `cicada-app` has no way for a host to tie a background task to the node's life.

## What Changes

- A background task hook in `cicada-app`, such as `App::task(|system, stop| async { ... })`: the node starts it with
  the node, passes a stop signal at shutdown and waits for it to end in good order, and when the task ends or panics
  early the node either turns `/ready` to 503 or stops with an error (Factory slightly prefers stopping, so its
  supervisor restarts it).

To decide in the design: stop the node or turn `/ready` to 503 (or a per-task choice); the time a task gets at
shutdown; `serve` and `worker` both or `serve` only; how the task reaches the system; whether it needs an ADR.

## Out of scope

Supervision of tasks inside the node (restarts). Tasks for operator commands.

## Order

After `containers-guide-doctests`, before `container-output-store`.

## Impact

`crates/cicada-app` (`App`, `serve`, `worker`), its tests; the finishing gate runs E2E. Additive, in the 0.22.0 batch.
