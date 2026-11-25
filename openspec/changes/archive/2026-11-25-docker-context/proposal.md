# Proposal

## Why

`DockerRuntime::connect()` reads only `DOCKER_HOST`, else the local socket `/var/run/docker.sock`. On a Mac with
OrbStack, Colima or Docker Desktop and no symlink at that path, it finds no Docker, while the `docker` CLI works
through its current context (here: context `orbstack`, endpoint `unix:///Users/<user>/.orbstack/run/docker.sock`).
Factory works around it in its `scripts/dev.sh` with `docker context inspect` and `connect_to`, and so did this repo's
E2E class after a reboot. Low priority: it affects developer machines, not servers (request from the Factory session,
2026-11-25; Factory issue 0019).

## What Changes

- When `DOCKER_HOST` is not set and the default socket is missing, `connect()` reads the endpoint the way the Docker
  CLI does: the context named by `DOCKER_CONTEXT`, else `currentContext` in `config.json` (the folder rule of
  `container-registry-auth`: `DOCKER_CONFIG`, else `$HOME/.docker`), then
  `contexts/meta/<sha256 of the name>/meta.json`, `Endpoints.docker.Host`.
- Only `unix://` and `tcp://` without TLS, as `connect_to` takes today. The error names the context and the endpoint it
  tried. The context `default`, or none, keeps today's behaviour.
- The `DockerRuntime` rustdoc ("Docker CLI contexts are not read") changes with it.

To decide in the design: whether a set `DOCKER_CONTEXT` wins over an existing default socket (the CLI lets it win);
whether `docker_config(dir)` applies (it is set after `connect`, so probably not); TLS contexts (`SkipTLSVerify`, the
`tls/` folder): refuse with a clear error, or support; the E2E class dropping its own context lookup.

Factory's view (2026-11-25), input for the design: `DOCKER_HOST` wins over everything; a set `DOCKER_CONTEXT` wins
over an existing default socket, as in the CLI; `currentContext` from `config.json` is read only when the socket is
missing, so servers keep today's behaviour; a TLS context (TLS files or `SkipTLSVerify`) is refused in `connect()` with
an error that names the context and says to set `DOCKER_HOST`, with no silent fallback to the socket or plain `tcp`.

## Out of scope

`ssh://` and TLS endpoints, unless the design takes TLS. Kubernetes or other context endpoints. Switching contexts
after connect.

## Order

First in the line, before `watch-absent-target`.

## Impact

`crates/cicada-containers` (`DockerRuntime::connect`, the config-folder helper of `src/docker/auth.rs`), its unit tests
and the E2E class. Additive (a connect that failed now finds the daemon): a patch, 0.21.2.
