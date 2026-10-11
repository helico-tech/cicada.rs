---
status: done
created: 2026-10-11
---

# 01. Crate, container spec and the Engine seam

## Goal

A crate `cicada-docker` with the container spec, the observed container and the `Engine` trait, implemented by `DockerEngine` on bollard.

## Acceptance criteria

- `ContainerSpec` (image, cmd, entrypoint, env, labels, ports, binds, network, restart policy, pull policy, stop timeout) with a stable spec hash.
- `Engine`: inspect, pull, image exists, create, start, stop, remove, logs (stream), all by container name.
- `DockerEngine` connects with Docker's defaults (`DOCKER_HOST` or the local socket).
- Doc comments on every public item; clippy clean.

## Plan

Module `spec` (`ContainerSpec` with builder methods, `PortBinding`, `RestartPolicy`, `PullPolicy`; camelCase JSON;
`hash()` = SHA-256 of the JSON), module `engine` (`Engine` trait with async-trait, `ContainerInfo`, `ContainerStatus`,
`LogOptions`, `EngineError`), module `docker` (`DockerEngine` on bollard 0.21: 404/409 map to `NotFound`/`Conflict`,
304 on start/stop and 404 on remove are success; an untagged image is pulled as `:latest`).

## Progress log

- 2026-10-11: Built. `Engine::stop` has no timeout parameter: Docker uses the container's own stop timeout (the
  spec's `stopTimeoutSeconds`). Unit tests: spec hash and wire form, Docker's "never" time. clippy clean.

## Follow-ups
