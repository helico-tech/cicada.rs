---
status: todo
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

## Progress log

## Follow-ups
