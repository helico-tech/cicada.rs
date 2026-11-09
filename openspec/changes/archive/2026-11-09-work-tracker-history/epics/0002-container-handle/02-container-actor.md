---
status: done
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

`ContainerActor { engine: Arc<dyn Engine>, prefix, check_every }`. Each turn: validate the name, run the command
(inspect first; a container without the handle's `cicada.uri` fails permanently), then `observe` (never fails; errors
go to `checkError`) and re-arm one `Check` (cancel the previous pending one, `Retention::REMOVE_ON_DONE` so polls do
not fill the outcomes table). `Remove` ends with `turn.stop()`. Tests in `tests/container_actor.rs` on a fake engine
(`tests/support`).

## Progress log

- 2026-10-11: Built; 13 tests on `TestActors` (virtual time): apply/pull/create/start, repeat apply, spec change,
  retried apply after a failed start, foreign container, exit with code and last lines (read once per run), Docker
  down during a check, stop/start, start after an outside removal, start without spec, remove, pull `never`, reserved
  labels and bad names.
- `Start` creates a missing container (from the last applied spec); `Check` never does.

## Follow-ups
