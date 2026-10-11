---
status: done
created: 2026-10-11
---

# 05. Sample service group and docs

## Goal

Let people try container handles on the sample node, and document them.

## Acceptance criteria

- Sample service: an opt-in setting that hosts `ContainerActor` and mounts the logs router next to the API.
- README section, architecture crate table, AGENTS.md map and test command.
- Follow-up issues for the events watcher, a console logs panel and several Docker hosts.

## Plan

`CONTAINERS=true` (default `false`) instead of a group: the catalog's groups are what `HOSTED_TYPES=all` and the
contract snapshots cover, and Docker must not be needed for `all`. The node connects with `DockerEngine::connect()`,
hosts `ContainerActor`, lists `actor://containers/{name}` in `GET /`, and merges `container_logs()` into the
authenticated routes. Docs: README section and tables, architecture crate table, AGENTS.md map and test command.

## Progress log

- 2026-10-11: Built. Tried by hand on port 8095 with `CONTAINERS=true HOSTED_TYPES=none`: `PUT` the handle, `POST`
  an `Apply` with `alpine:3.20`, state showed `running`, the logs route returned the output, `Remove` deleted the
  container and the handle. Found and fixed: Docker reports exit code 0 while a container runs; `exitCode` is now
  recorded only once it has ended (asserted in the Docker test).

## Follow-ups

- [0800 Docker events watcher](../../issues/0800-docker-events-watcher.md)
- [0801 Container logs in the console](../../issues/0801-console-container-logs.md)
- [0802 Several Docker hosts](../../issues/0802-several-docker-hosts.md)
