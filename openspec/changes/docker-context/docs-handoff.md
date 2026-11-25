# Docs handoff for docker-context

Version: 0.21.2. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

`DockerRuntime::connect()` finds Docker through the Docker CLI's context, so it works on a Mac with OrbStack, Colima or
Docker Desktop and no `/var/run/docker.sock`, where it used to fail with `DaemonUnavailable` while `docker` worked.
No public item is added or removed; only the behaviour of `connect()` and the rustdoc of `DockerRuntime`, `connect()`
and `connect_to` change.

The order, as the CLI's:

```text
DOCKER_HOST set            -> that host (as before)
DOCKER_CONTEXT set         -> "default": the local socket; other: that context's endpoint, even when the socket exists
local socket exists        -> the local socket (as before)
currentContext in config   -> that context's endpoint ("default", none or no config.json: the local socket, as before)
```

- The config folder is `DOCKER_CONFIG`, else `$HOME/.docker`, as for pulls. `docker_config(dir)` does not steer it: it
  is set after `connect`.
- A context's endpoint is `Endpoints.docker.Host` of `contexts/meta/<sha256 of its name>/meta.json` in that folder, as
  the CLI stores it, and is connected as `connect_to(host)` connects it. `connect()` still does not contact the daemon.
- An empty `DOCKER_CONTEXT` counts as unset.
- Refused with `Other`, naming the context and whether its name came from `DOCKER_CONTEXT` or `currentContext`, never
  falling back to the socket or plain `tcp`: no meta file, no docker endpoint, broken JSON, `DOCKER_CONTEXT` with no
  config folder (neither `DOCKER_CONFIG` nor `HOME`), TLS (`SkipTLSVerify: true` or a `contexts/tls/<sha256>/docker`
  folder), and schemes other than `unix://`, `tcp://` and `http://` (`ssh://`, `npipe://`, `https://`). The TLS
  message: `connect to docker: context <name> from <source> uses TLS, which connect() does not support; set
  DOCKER_HOST or use connect_to`.
- A context whose unix socket file is missing fails with the retryable `DaemonUnavailable`:
  `connect to <endpoint> of context <name> from <source>: ...`.

Proved by the unit tests in `crates/cicada-containers/src/docker/context.rs` (each row of the order, `default`, an empty
`DOCKER_CONTEXT`, a `currentContext` ignored while the socket exists, the sha256 path of `orbstack`, missing contexts
from both sources, no docker endpoint, broken JSON, no config folder, `SkipTLSVerify`, a TLS folder, `ssh`/`npipe`/
`https`, a missing or broken `config.json`), `a_context_whose_socket_is_missing_is_daemon_unavailable_naming_both` and
`a_context_endpoint_is_connected_as_connect_to` in `src/docker/mod.rs`, and the E2E class
`e2e/tests/suite/container_docker.rs`, whose `connect()` helper now calls `DockerRuntime::connect()` alone and passes
on a machine without `/var/run/docker.sock` (current context `orbstack`).

## 1. The container runtime: finding Docker

**Where.** No guide covers `cicada-containers` yet; the page the docs session picks for the container runtime (with
`The runtime's machine` from `container-info` and `Private images` from `container-registry-auth`), under a heading
such as `Finding Docker`.
**Text.** `DockerRuntime::connect()` uses the Docker the `docker` CLI uses: `DOCKER_HOST`, else the context
`DOCKER_CONTEXT` names, else `/var/run/docker.sock` when it exists, else the CLI's current context (`docker context
use`). So on a Mac with OrbStack, Colima or Docker Desktop it works without a symlink at `/var/run/docker.sock`. For a
setup `connect()` does not take (a TLS or `ssh` context, or a host that knows its endpoint), use `connect_to`:

```rust
let runtime = DockerRuntime::connect()?;                              // as the docker CLI finds it
let runtime = DockerRuntime::connect_to("tcp://build-host:2375")?;    // an endpoint you give
```

(A sketch from the rustdoc of `DockerRuntime`; no doctest exists for this crate in the guides yet.)
**Caveats.** A server with a local socket keeps using it whatever `config.json` names as `currentContext`; only
`DOCKER_CONTEXT` or `DOCKER_HOST` move it. So on a machine with both a working socket and another current context,
`connect()` uses the socket while `docker` uses the context: set `DOCKER_CONTEXT` to make them agree. TLS contexts
are refused, not used without verification: set `DOCKER_HOST` to a plain endpoint or use `connect_to`. A context is
read once, at `connect()`; switching contexts later does not move a runtime. `docker_config(dir)` sets only where
pulls find logins, not where `connect()` finds contexts.

## Blocking

None.

## Status

Ran `cargo test -p cicada-containers --all-features`, the Docker E2E class on a machine without `/var/run/docker.sock`
(30 passed) and the finishing checks of tasks.md; see the review request for
results. All text above is proposed; no protected prose was edited, and no doctest, link, guarantee row or skill route
needs new prose.
