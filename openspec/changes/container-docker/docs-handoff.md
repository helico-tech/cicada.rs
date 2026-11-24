# Docs handoff for container-docker

Version: 0.20.7 (the unreleased batch with `container-runtime`). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

`cicada-containers` gets a Docker backend behind the feature `docker`: `DockerRuntime`, a `ContainerRuntime` over the
Docker Engine API through bollard 0.21 (Unix socket, Windows pipe and plain TCP; no TLS, no SSH, so no rustls). No
public item names a bollard type.

- `DockerRuntime::connect() -> Result<DockerRuntime, ContainerError>`: sync, does not contact the daemon. It reads
  `DOCKER_HOST` (`unix://`, `tcp://`, `http://`) or uses the local socket. A missing socket file is
  `DaemonUnavailable`; an unsupported scheme (`https`, `ssh`) is `Other`. The API version is agreed on the first call
  and kept; a failed agreement is retried on the next call. Docker 25 (API 1.44) or later; an older daemon is `Other`.
- `DockerRuntime::connect_to(host: &str) -> Result<DockerRuntime, ContainerError>`: the same with an explicit host
  in `DOCKER_HOST`'s form (`unix:///path`, `tcp://h:p`, `http://h:p`), for a host that finds its daemon another way,
  such as the current Docker CLI context. A missing socket file is `DaemonUnavailable`, another scheme `Other`.
  `connect()` is unchanged. Added at the owner's request so the E2E class can follow the CLI's context without a
  process-wide `DOCKER_HOST`; it takes a string, so no bollard type enters the API.
- Containers are found by their exact name: Docker would also accept an ID prefix, so the backend inspects first and
  treats another name as missing.
- `create` keeps an existing container (no image check), refuses an empty env name or one with `=` (`InvalidSpec`),
  pulls a missing image (a bare repository as `:latest`), and is `ImageNotFound` when the registry has none. No
  registry login: a private image must be local.
- `start` acts only on `Created`; `stop` only on `Running` (grace rounded up to seconds); `remove` is forced, with
  anonymous volumes. `stop` sends the image's stop signal, as `docker stop` does: SIGTERM unless the image names
  another (`postgres` names SIGINT, so a stopped postgres container exits with 130, not 143).
- `copy_in`: only a `Created` container; absolute paths without `..`; a path on a read-only mount is refused; with
  `read_only_root`, only below a volume or bind mount (a tmpfs does not count: Docker mounts it at start). Files are
  owned by root (uid and gid 0) with their mode.
- Status mapping: `created` → `Created` (or `Failed` after a refused start), `running`/`paused`/`restarting` →
  `Running`, `exited` → `Exited`, unless out of memory (`Failed`, "out of memory") or with a daemon error; `dead` →
  `Failed`.
- Error mapping: no connection or a timeout → `DaemonUnavailable`, 409 → `Conflict`, 400 → `InvalidSpec`, a missing
  image → `ImageNotFound`, else `Other`. Messages read `<action> <name>: <Docker's message>`.
- Secret environment goes only into the create call's JSON body; error messages of `create` have every secret value
  replaced by `***`. bollard itself logs request bodies, secret values included, at `debug` under the target `bollard`.
- `sweep` removes containers, then networks, then volumes; anything Docker refuses (a network with active endpoints, a
  volume in use) is skipped with Docker's message.
- The fake (`testing`) now matches Docker: its `copy_in` also refuses a path with `..` and a path on a read-only
  mount, and `CopyFile`'s `Debug` shows path, mode and length, not contents.

Proved by the unit tests in `crates/cicada-containers/src/docker/{mod,map}.rs` (mappings, scrub, tars, mount check,
a socket without a daemon) and by the E2E class `ContainerDockerTest` in `e2e/tests/suite/container_docker.rs`, run
tenth. No worked example in `examples/`: it would need a daemon.

## 1. The feature in the map

**Where.** `AGENTS.md`, `Map`: the row of `crates/cicada-containers` (added by the `container-runtime` handoff).
**Text.** Add: "`DockerRuntime` behind the feature `docker` (bollard, no TLS); `docker/map.rs` holds its pure
mappings".
**Caveats.** None.

## 2. Choosing the crate

**Where.** `docs/how-to/install.md`, `Choose the crates`: the `cicada-containers` row.
**Text.** "Run containers" | "`cicada-containers` from the same tag with `features = [\"docker\"]` for
`DockerRuntime`; `features = [\"testing\"]` in dev-dependencies for `FakeRuntime`".
**Caveats.** The `container-runtime` handoff said "no Docker backend before `container-docker`": that caveat goes.

## 3. Running the Docker backend

**Where.** The crate's guide when one exists (`container-actor` may bring it); until then `README.md`, `Layers`, the
`cicada-containers` row may point at the crate's rustdoc, which says all of this on `DockerRuntime`.
**Text.** For hosts:
- `DOCKER_HOST` is read; Docker CLI contexts (`docker context use`) are not. When the daemon has no socket at the
  default place (OrbStack without its `/var/run/docker.sock` link, Colima, rootless Docker), set
  `DOCKER_HOST=$(docker context inspect -f '{{.Endpoints.docker.Host}}')`, or pass that endpoint to
  `DockerRuntime::connect_to`. Docker 25 or later.
- `stop`'s exit code depends on the image's stop signal: 143 after SIGTERM, 130 after SIGINT (`postgres`), 137 when
  the grace period runs out.
- The backend needs the host's Tokio runtime.
- `copy_in` on a container with `read_only_root` writes only below a volume or bind mount, not a tmpfs.
- A `tmpfs` is `noexec` unless its `exec` is set (Docker's default `noexec,nosuid,nodev`).
- Keep `RUST_LOG`'s `bollard` target at `info` or above: at `debug`, bollard logs request bodies, secret environment
  included. Example: `RUST_LOG=debug,bollard=info`.
- A private image must be pulled beforehand: the backend does no registry login.
**Caveats.** The Docker socket is root on the host; the docs should not suggest mounting it into untrusted containers.

## 4. The E2E suite

**Where.** `e2e/README.md`: the list of classes, and cleanup.
**Text.** The tenth class, `ContainerDockerTest`: `DockerRuntime` against the daemon. It uses `postgres:18` (pulled by
the stack) and pulls `busybox:1.37.0` (removed first when no container uses it), and needs registry access for the
pull and the unknown-tag test. Everything it makes is named `cicada-rs-e2e-ctr-*` and labelled
`cicada-rs-e2e=containers`; its hooks sweep the label and the suite's teardown removes it with the CLI. By hand:
`docker rm -f -v $(docker ps -aq --filter label=cicada-rs-e2e=containers)`, then
`docker network rm $(docker network ls -q --filter label=cicada-rs-e2e=containers)` and
`docker volume rm $(docker volume ls -q --filter label=cicada-rs-e2e=containers)`.
**Caveats.** Run only this class with `CICADA_E2E_ONLY=ContainerDockerTest` (the stack still comes up first). The
class reaches the same daemon as the rest of the suite: `DOCKER_HOST` when it is set, else the endpoint of the CLI's
current context (`docker context inspect`), passed to `DockerRuntime::connect_to`. No `DOCKER_HOST` is needed.

## Blocking

None: no doctest, link, guarantee row or skill route depends on new prose.

## Status

Proposed text only; no protected file was edited. Checks: see the build report of this change.
