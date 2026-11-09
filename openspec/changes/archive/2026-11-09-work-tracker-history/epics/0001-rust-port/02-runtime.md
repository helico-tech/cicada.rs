---
status: done
created: 2026-10-07
---

# 02. Runtime

## Goal

Runtime: Actor trait, Turn, plugins, executor, ActorSystem with dispatch loop and operations.

## Acceptance criteria

- Turns follow the outcomes table; lease guard, heartbeat, watchdog, timeout and fenced commit with retry as in Kotlin `TurnExecutor`.
- `ActorSystem` offers start/stop, `run_due_turns` and the fourteen operations in-process.

## Plan

Port `TurnExecutor`, `ActorSystem`, `TurnScopeImpl`, `Plugins` idiomatically on tokio.

## Progress log

- 2026-10-07: done in the initial build.

## Follow-ups

- (none yet)
