---
status: done
created: 2026-10-11
---

# Epic 0002: Container handles

## Goal

Actors that look after Docker containers: run one with a spec, see its status, read its logs, stop and remove it.
Design: [ADR 0007](../../../../../../docs/adr/0007-container-handles.md).

## Scope

A new crate `cicada-docker` with the `Engine` seam and its bollard implementation, the `ContainerActor` type, a logs
router, tests on a fake engine and against a real daemon, the sample service and docs. Not in scope: other process
kinds (plain processes, Kubernetes jobs), a Docker events watcher, a console logs panel, several Docker hosts.

## Done when

All stories are done, `cargo test --workspace` and `CICADA_DOCKER=1 cargo test -p cicada-docker` pass, CI runs the
Docker tests, and the branch is merged and pushed.

## Stories

- [01 Crate, container spec and the `Engine` seam with a bollard engine](01-engine.md) — done
- [02 `ContainerActor`: commands, status polling, tests on a fake engine](02-container-actor.md) — done
- [03 Logs router](03-logs-router.md) — done
- [04 Tests against a real Docker daemon, in CI](04-docker-tests.md) — done
- [05 Sample service group and docs](05-sample-and-docs.md) — done
