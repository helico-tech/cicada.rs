# Design

`DockerRuntime::connect()` finds no Docker on a Mac whose Docker (OrbStack, Colima, Docker Desktop) listens on its own
socket, while the `docker` CLI finds it through its current context. `connect()` reads the context the way the CLI
does, in the order Factory asked for, so `docker` and Cicada agree. Nothing else changes.

```text
DOCKER_HOST set            -> that host (as today)
DOCKER_CONTEXT set         -> "default": the local socket; other: that context's endpoint, even when the socket exists
local socket exists        -> the local socket (as today)
currentContext in config   -> that context's endpoint ("default" or none: today's error)
```

## Decisions

1. **The order above.** `DOCKER_HOST` wins over everything, as in the CLI. A set `DOCKER_CONTEXT` wins over an
   existing local socket: whoever sets it means it, and a difference from `docker` is the bug being fixed. The
   `currentContext` of `config.json` is read only when the local socket (`/var/run/docker.sock`) is missing, so a
   server with a socket behaves as today whatever its `config.json` says (Factory's view). Not taken: the CLI's full
   rule, where `currentContext` also wins over the socket (it would change servers' behaviour in a patch).
2. **The config folder of `container-registry-auth`, without `docker_config`.** `DOCKER_CONFIG`, else `$HOME/.docker`,
   through `auth::config_dir(None)`. `docker_config(dir)` is a builder method called after `connect`, so it cannot
   steer it; a host that knows its endpoint uses `connect_to`. Not taken: a `connect_with_config(dir)` (nobody asked).
3. **The context's endpoint, as the CLI stores it.** `contexts/meta/<sha256 of the name, lowercase hex>/meta.json` in
   that folder, field `Endpoints.docker.Host`, read with `serde_json` and `sha2` (already a workspace dependency).
   A context that has no meta file, no docker endpoint or broken JSON fails with `Other`, naming the context and
   where its name came from (`DOCKER_CONTEXT` or `currentContext`): a retry cannot help. An unreadable or missing
   `config.json`, or one without `currentContext`, gives today's error for the missing socket. A context named by
   `DOCKER_CONTEXT` with no config folder (neither `DOCKER_CONFIG` nor `HOME` set) fails with `Other` like a missing
   meta file. An empty `DOCKER_CONTEXT` counts as unset, as in the CLI and `config_dir` (design review).
4. **TLS and other schemes are refused, never bypassed.** A context with `SkipTLSVerify: true`, or with a
   `contexts/tls/<sha256>/docker` folder, fails with `Other`: "context <name> uses TLS, which connect() does not
   support; set DOCKER_HOST or use connect_to". So does an endpoint scheme other than `unix://`, `tcp://` and
   `http://` (`ssh://`, `npipe://`), naming the context and the endpoint. No fallback to the socket or plain `tcp`.
   Not taken: TLS support (a TLS library in the `docker` feature, which the spec rules out today).
5. **Connected as `connect_to` connects.** The endpoint goes to the same code as `connect_to(host)`, so a unix
   endpoint whose socket file is missing fails with `DaemonUnavailable`, and the message names the context and the
   endpoint. It still does not contact the daemon.
6. **A pure resolver.** The order is a function of the two variables, the config folder and whether the local socket
   exists, passed in as values, so unit tests never change the process environment. `connect()` gathers them and
   calls it.

## Tests

- Unit (`src/docker/context.rs`): each row of the order; `DOCKER_CONTEXT=default` with and without a socket; an empty
  `DOCKER_CONTEXT`; a `currentContext` ignored while the socket exists; the meta path for a known name (sha256 of
  `orbstack`); a missing context from `DOCKER_CONTEXT` and from `currentContext`, no docker endpoint, broken JSON;
  `DOCKER_CONTEXT` without a config folder; `SkipTLSVerify`, a TLS folder, `ssh://`; a missing `config.json`; a unix
  endpoint whose socket file is missing (`DaemonUnavailable`, naming the context and the endpoint).
- E2E (`e2e/tests/suite/container_docker.rs`): its `connect()` helper calls `DockerRuntime::connect()` alone, without
  its own `docker context inspect`; the whole class passes on a machine without `/var/run/docker.sock`. The CLI
  helper for the registry test keeps passing `-H` with the context's host, as its own `--config` folder holds no
  context. The helper's doc comment says that on a machine with both a working local socket and another current
  context the backend uses the socket while the CLI uses the context (design review, note 2).

## Docs

Nothing breaks. The handoff gives the container page the order above, the TLS refusal and `connect_to` for other
setups. The rustdoc of `DockerRuntime`, `connect()` and `connect_to` ("Docker CLI contexts are not read") changes with the code.
