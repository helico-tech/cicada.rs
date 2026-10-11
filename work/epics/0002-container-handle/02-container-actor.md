---
status: todo
created: 2026-10-11
---

# 02. ContainerActor

## Goal

The handle: `ContainerHandle` (`actor://containers/{name}`) and `ContainerActor` with `Apply`, `Start`, `Stop`, `Remove` and `Check`, as ADR 0007 describes.

## Acceptance criteria

- Every command inspects first and is harmless when repeated; a turn that runs again makes no second container.
- `Apply` replaces a container whose spec hash differs; a container of another `cicada.uri` fails the command permanently.
- `Check` records status, exit code, times and the last lines at exit; a Docker error is recorded and polling continues.
- Exactly one `Check` is pending while the handle exists; `Remove` removes the container and the handle.
- Tests on `TestActors` (virtual time) with a fake engine cover all of the above.

## Plan

## Progress log

## Follow-ups
