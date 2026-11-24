# Design

Decided with the owner and the change loop, 2026-11-24. Implements the trait of `container-runtime` (its design and
spec are the reference for the repeat rules, `ContainerStatus` and `ErrorKind`). Reference: cicada-code-factory
`src/runner/docker.rs`.

## Decisions

1. **`DockerRuntime::connect() -> Result<Self, ContainerError>`, sync, no I/O.** It calls bollard's
   `Docker::connect_with_defaults()`: `DOCKER_HOST` when set (`unix://`, `tcp://`, `http://`), else the local socket.
   Not `connect_with_local_defaults()`: it ignores a `tcp://` `DOCKER_HOST`. An unsupported scheme (`https`, `ssh`) is
   `Other`. No bollard type in the public API, so a bollard upgrade is not a breaking change of ours. Not a host
   argument or TLS: no consumer needs a remote daemon; it can come later as an additive constructor.
2. **The API version is negotiated on the first call** and kept in a `tokio::sync::OnceCell`; a failed negotiation is
   not kept, so the next call tries again. bollard asks for API 1.53, which older daemons refuse. The floor is API 1.44
   (Docker 25): one create with several networks needs it; an older daemon fails with `Other`. Not at `connect`: a host
   would fail to start while the daemon is down, where a retryable error is wanted.
3. **Dependencies behind `docker`**, added to `[workspace.dependencies]`: `bollard = { version = "0.21.1",
   default-features = false, features = ["http", "pipe"] }` (0.21.1 is current; `pipe` is the Unix socket and the
   Windows pipe, `http` plain TCP; this equals bollard's defaults, named so a new default never adds TLS; no `ssl`,
   `aws-lc-rs` or `ssh`, so no rustls), `tar = { version = "0.4.46", default-features = false }` (no `xattr`),
   `futures` from the workspace (`0.3.34`; the pull and log streams), `tokio` from the workspace (the `OnceCell`; bollard needs the
   host's Tokio runtime anyway). New in `Cargo.lock`: bollard, bollard-stubs, hyperlocal, serde_repr, tar, filetime,
   and hyper-named-pipe on Windows targets; hyper 1.12, hyper-util 0.1.21 and tokio 1.54 are there already.
4. **Containers are found by exact name.** Docker resolves a name or an ID prefix, so `inspect("cafe")` can return
   another container. Every container method inspects first, treats a name other than `/<name>` as `Missing`, and
   acts on the full ID it got. Same for networks; volumes have no IDs. Not a call by name: a short name could stop,
   start or remove a stranger's container.
5. **`create`**: first inspect the container by exact name (decision 4): if it exists, `Ok`, with no image check, so
   a repeat succeeds even when the image is gone. Then refuse an empty env name or one with `=` (`InvalidSpec`, the
   name only); `inspect_image`, on 404 pull
   it (`create_image`, the stream read to the end); then `create_container`. A 404 from the pull, or a stream error
   saying `not found`, `manifest unknown` or `does not exist`, is `ImageNotFound` (text matching: the stream has no
   code). A 409 on create is the existing container: `Ok`. No registry login: a private image must be local.
6. **`start`** acts only when the status is `Created`; anything else is `Ok` (Docker would rerun an exited container).
   A refused start (missing executable) maps by the error rules and leaves the container `Failed`, so a repeat is `Ok`.
   A 304 (started by someone else after the inspect) is `Ok`.
   **`stop`** acts only on `Running`: `stop_container` with `t` = grace rounded up to seconds, on a client whose
   request timeout is grace + 60 s (bollard's 120 s would cut a long grace); 304 is `Ok`. **`remove`**: force, with
   anonymous volumes (`v`); 404 and 409 (removal in progress) are `Ok`.
7. **`copy_in`** checks `Created` (else `Conflict`), refuses a relative path or a `..` part (`InvalidSpec`), and uploads
   in-memory tars: each file under its path relative to the upload directory, its mode, uid and gid 0, mtime 0;
   Docker makes missing parents. Without `read_only_root`, one tar at `/`. With it, Docker checks the upload
   directory, not each entry, so the backend refuses a path outside every volume or bind mount target with
   `InvalidSpec` before any call, then uploads one tar per mount target. A tmpfs does not count: Docker mounts it only
   at start, so a file copied there would land under it and be hidden (host docs). The spec it needs comes from the
   container's inspect (`ReadonlyRootfs`, `Mounts`), so `copy_in` keeps its signature.
8. **`logs`**: `follow` off, stdout and stderr, `tail` = the count; the chunks are joined as bytes, then lossy UTF-8
   once, so a character split over two chunks survives. **`list`** inspects each listed container for its status
   (one that vanished in between is left out). Not parsing the summary's `Exited (137) …` text.
9. **Networks and volumes.** `ensure_network`: inspect; differing `internal` is `Conflict`; on 404 create a bridge
   network with the labels; a 409 race inspects and compares again. `ensure_volume`: Docker's create returns an
   existing volume unchanged, so one call.
10. **Secrets.** `env` and `secret_env` go together as `K=V` in the JSON body of the create call. An error message
    built from a create call has every non-empty secret value replaced by `***`. The backend logs nothing itself. bollard logs
    request and response bodies, secret values included, at `debug` under the target `bollard`; the backend cannot
    prevent that, so the docs say to keep that target at `info` (handoff).

## Mappings

Spec → create body: `entrypoint` → `Entrypoint`; `command` → `Cmd`; `workdir`, `user`, `labels`, `env` as named;
`cpu_millis` → `NanoCpus` (× 1 000 000); `memory_bytes` → `Memory` and `MemorySwap` the same (no swap, so the limit
holds); `mounts` → typed `HostConfig.Mounts` (`volume` or `bind`, `Target`, `ReadOnly`), not `Binds` strings;
`Networking::Default` → nothing, `None` → `NetworkMode: none`, `Attach` → `NetworkMode` = the first name and
`EndpointsConfig` for each with its `Aliases`. Hardening: `read_only_root` → `ReadonlyRootfs`;
`drop_all_capabilities` → `CapDrop: ["ALL"]`; `no_new_privileges` → `SecurityOpt: ["no-new-privileges"]`; `tmpfs` →
`Tmpfs {target: "size=<bytes>[,mode=<octal>][,uid=<n>][,gid=<n>][,exec]"}` (Docker adds `noexec,nosuid,nodev`
unless `exec`); `init` → `Init`; `pids_limit` → `PidsLimit`.

State → `ContainerStatus`: 404 or another name → `Missing`; `created` → `Created`, or `Failed { code, reason: Error }`
when its error is set (a refused start); `running`, `paused`, `restarting` → `Running`; `exited` and `removing` →
`Exited { code }`, unless `OOMKilled` (`Failed`, "out of memory") or a non-empty error (`Failed` with it); `dead` →
`Failed`.

Errors → `ErrorKind`: I/O, hyper and socket errors and request timeouts → `DaemonUnavailable`; 409 → `Conflict`; 400 →
`InvalidSpec`; an image 404 → `ImageNotFound`; anything else (500 too) → `Other`. A missing container is not an error
code: `inspect` says `Missing`, `stop` and `remove` are `Ok`, `start`, `copy_in` and `logs` are `Conflict` (the trait).
Messages read `<action> <name>: <Docker's message>`.

**Sweep**: list by label (`all`), remove each container (as `remove`); then networks, then volumes, each listed by
label and removed. In use is Docker's own refusal (403 or 409, "active endpoints", "volume is in use"), not a check
before, so nothing races; any refusal but `DaemonUnavailable`, of a container, network or volume, is a skip with
Docker's message, and the sweep goes on.

## Tests

Unit tests in the crate, without Docker, on the pure parts: the create body for a full spec (every row above), the
state and error mappings, the secret scrub, the tar entries, path refusal, and a call to a missing socket
(`DaemonUnavailable`, through a crate-private constructor). The Docker test is a class `ContainerDockerTest` in
`e2e/tests/suite/container_docker.rs`, run tenth, so the gate's E2E kind runs it with no policy change. Image:
`postgres:18`, already pulled by the stack, with GNU tools; `busybox:1.37.0` only for the pull, removed first when no
container uses it. Everything it makes is named `cicada-rs-e2e-ctr-*` and labelled `cicada-rs-e2e=containers`;
`before_all` and `after_all` sweep that label, and the suite's teardown removes it with the CLI after a panic or
Ctrl-C. It asserts, against the daemon: a repeated create keeps the first spec; a second start of an exited container
leaves its code and runs nothing; `stop` gives 143 with `init` and 137 without (PID 1 ignores SIGTERM); stop of a
created container leaves it `Created`; stop and remove of a missing one and a repeated remove are `Ok`; an ID prefix
is `Missing` and survives `remove`; `copy_in` before start (contents and mode read back by the container), and
`Conflict` when running or missing; `start` and `logs` of a missing one are `Conflict`; OOM (`tail /dev/zero`, 32 MiB)
is `Failed` "out of memory"; a missing executable fails `start` with `InvalidSpec`, then `Failed`, then a repeat `Ok`;
a missing tag is `ImageNotFound` with no container; a pulled image; a hardened container (read-only root, no
capabilities, `NoNewPrivs`, writable tmpfs) and its secret env, read from its logs; repeated ensures and an `internal`
conflict; a sweep that removes the `run=a` set, skips a network used by an unlabelled container and leaves `run=b`.

## Gate and version

`feature-builds.sh`: `cargo check -p cicada-containers --features docker`. `dependency-trees.sh`:
`cicada-containers` without features has no `bollard`; with `docker` it builds `bollard` and no `rustls` or
`native-tls`. `cicada-e2e` depends on the crate with `docker` (allowed for tests, ADR 0056), so `cargo test
--workspace` compiles bollard and runs the backend's unit tests. Additive: patch, in the unreleased 0.20.7 with
`container-runtime`. No guarantee, wire format, schema or command line changes; no ADR.

## Docs it breaks

None: no doctest, link, guarantee row or skill route. For the docs session: the `docker` feature and `DockerRuntime`
in the AGENTS.md map row and `docs/how-to/install.md#choose-the-crates`; in `e2e/README.md`, the tenth class, its
images and label, and its cleanup (`docker rm -f $(docker ps -aq --filter label=cicada-rs-e2e=containers)`, then the
same for networks and volumes). For hosts: `DOCKER_HOST` is read but Docker CLI contexts are not; Docker 25 or later;
`copy_in` on a read-only root only to a mount; keep `RUST_LOG`'s `bollard` target at `info` (decision 10); a `tmpfs` is
`noexec` unless its `exec` is set.
