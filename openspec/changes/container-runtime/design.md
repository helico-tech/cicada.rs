# Design

Decided with the owner, 2026-11-24 (labels C-ADR, C-SPLIT, C-BACKEND, C-COPY, C-STREAM, C-NET, C-SPEC, C-PULL, C-ERR).
References: cicada-code-factory `src/runner/` (bollard `Runner` and fake), brood `crates/brood-agent/src/container.rs`
(docker-CLI `ContainerRuntime` and fake).

## Decisions

1. **A new crate `crates/cicada-containers`, under ADR 0056** (C-ADR). Two consumers need the same layer, so it lives
   here, outside the core; no library crate depends on it (examples and tests may, ADR 0056). Not a crate of its own repo: a second release line.
   Not in `cicada-actors`: it would pull container types into every user of the ready-made actors.
2. **This change is the trait, its types, its error and a fake; no backend** (C-SPLIT). `container-docker` adds the
   bollard backend (C-BACKEND), its image pull (C-PULL) and its Docker test in the E2E kind. The trait names no bollard
   type, so another backend fits later. Not one change: too many tasks, and the trait gets its own review.
3. **No dependency on `cicada`.** The layer is about Docker, not actors. `container-actor` adds the dependency when
   the actor joins the crate.
4. **Dependencies: `async-trait` and `thiserror` only.** `async-trait`, as for the store and plugins (ADR 0002), so
   `Arc<dyn ContainerRuntime>` works: a host picks the fake or Docker at run time. Not native `async fn` in traits:
   not object-safe. No `serde`: a `ContainerSpec` holds secrets and must not be stored as is (factory keeps the spec
   out of actor state); `container-actor` gives its state and contract its own types. No `tokio`: the fake answers at
   once, so it needs no timer and runs under paused time.
5. **One error type** (C-ERR): `ContainerError { kind: ErrorKind, message: String }` with `ErrorKind` =
   `DaemonUnavailable` (retryable), `Conflict`, `ImageNotFound`, `InvalidSpec`, `Other`, and `is_retryable()`, true
   only for `DaemonUnavailable`; a caller may still retry the others (the actor counts errors in a row). `Conflict`
   means the call does not fit the current state: `start`, `copy_in` or `logs` on a missing container, `copy_in` on a
   container that is not `Created`, or `ensure_network` on an existing network with another `internal` flag. Not a `NotFound` kind: `inspect` reports
   `Missing`, and `stop` and `remove` are `Ok` on a missing container. Not `anyhow`: the actor must tell a retry from a
   permanent failure.
6. **Every call is safe to repeat.** `create` finds an existing container by name and returns `Ok` without comparing
   or replacing it: the name is the identity, and its owner (the actor) chooses it. `start` on a running, exited or
   failed container is `Ok` and does nothing, so a container runs at most once (a replayed turn never restarts an ended
   one). `stop` of a created container is `Ok` and leaves it `Created`; its owner removes it. `stop` and `remove` of a
   missing container are `Ok`. `ensure_*` find an existing network or volume by name; only a network's `internal` flag
   is compared, as it decides isolation (factory refuses that case too); a volume has nothing to compare. Not "create
   fails when it exists": every caller would match the error.
7. **`ContainerSpec` is `#[non_exhaustive]` with `pub` fields and `ContainerSpec::new(name, image)`**; a caller sets
   the rest by field. A new field later is additive. Fields: `name`, `image`, `entrypoint: Option<Vec<String>>`
   (factory replaces it), `command`, `workdir`, `env`, `secret_env`, `labels` (`BTreeMap`s), `user`, `cpu_millis`,
   `memory_bytes`, `mounts: Vec<Mount>` (volume or bind, target, read-only), `network: Networking` (`Default`, `None`,
   or `Attach(Vec<NetworkAttachment { name, aliases }>)`; factory's services need aliases), and the hardening (C-SPEC),
   all off by default: `read_only_root`, `drop_all_capabilities`, `no_new_privileges`, `tmpfs: Vec<Tmpfs { target,
   size_bytes }>`, `init`, `pids_limit`. `Debug` shows `secret_env` names only; a backend never puts its values on a
   command line. Every public struct and enum of the crate is `#[non_exhaustive]` the same way (`ErrorKind`,
   `ContainerStatus`, `Networking`, `Mount`, `NetworkSpec`, `VolumeSpec`, `CopyFile`, `Swept`), with a `new` where a
   caller builds one. Not a builder type: twice the API for the same thing.
8. **`ContainerStatus`**: `Missing`, `Created`, `Running`, `Exited { code: i64 }`, `Failed { code: Option<i64>,
   reason: String }` (did not start, out of memory, lost by the daemon). Two end states, so a caller can tell a
   program's own exit from the runtime's. A container ended by `stop` is `Exited` with the code the runtime reports
   (Docker: 143 after SIGTERM, 137 after the grace period): the stop was asked for, so its watcher sees an exit.
9. **`copy_in` takes files, not a tar** (C-COPY): `&[CopyFile { path, contents: Vec<u8>, mode: u32 }]` with absolute
   paths, meant for a created container before `start`. The backend builds the archive; the trait needs no tar crate
   and the fake records readable files. Repeating it writes the same files again. Not a raw tar: every caller would
   build one.
10. **No streaming** (C-STREAM): `logs(name, tail_lines)` returns the last lines of stdout and stderr together, as
    lossy UTF-8. brood's live output stays in brood. Not an optional attach method: the actor cannot use it, and it
    doubles the fake.
11. **Networks, volumes and a sweep** (C-NET): `ensure_network(&NetworkSpec { name, labels, internal })`,
    `ensure_volume(&VolumeSpec { name, labels })`, and `sweep(label)`, which removes every container, then network,
    then volume with that label, and returns what it removed and what it skipped. It is best effort: a network or
    volume still in use (by a container without the label) is skipped with its reason, and the sweep goes on; only
    `DaemonUnavailable` ends it with an error, and the next sweep finishes the work. Not single removals of networks
    and volumes: both consumers clean up by label. Not failing at the first busy resource: a later sweep would stop at
    the same one.

## The trait

```rust
#[async_trait]
pub trait ContainerRuntime: Send + Sync + 'static {
    async fn create(&self, spec: &ContainerSpec) -> Result<(), ContainerError>;   // may pull the image
    async fn copy_in(&self, name: &str, files: &[CopyFile]) -> Result<(), ContainerError>;
    async fn start(&self, name: &str) -> Result<(), ContainerError>;
    async fn inspect(&self, name: &str) -> Result<ContainerStatus, ContainerError>;
    async fn logs(&self, name: &str, tail_lines: u32) -> Result<String, ContainerError>;
    async fn stop(&self, name: &str, grace: Duration) -> Result<(), ContainerError>;
    async fn remove(&self, name: &str) -> Result<(), ContainerError>;            // forced
    async fn list(&self, label: &Label) -> Result<Vec<ContainerSummary>, ContainerError>; // name and status
    async fn ensure_network(&self, spec: &NetworkSpec) -> Result<(), ContainerError>;
    async fn ensure_volume(&self, spec: &VolumeSpec) -> Result<(), ContainerError>;
    async fn sweep(&self, label: &Label) -> Result<Swept, ContainerError>;
}
```

`Label { key, value }`; `Swept` lists the names removed per kind, and the skipped ones with their reason.

## The fake (feature `testing`)

`FakeRuntime`, in memory; clones share state. It models containers, networks and volumes by name with their labels
and states, following decision 6. A test scripts it: `exit_with(name, status)` moves a running container to its end
state, `set_logs(name, text)`, `fail_next(method, ErrorKind)` makes the next call of that method fail without effect,
`fail_always(method, kind)` until `clear_failures()`. It records every call in order (`calls()`, a `Call` enum whose
`Create` holds the spec); `CopyIn` keeps the files. No timers, so `container-actor` tests it on virtual time. The
module compiles under `cfg(any(test, feature = "testing"))`, so its own tests run in `cargo test --workspace`. Nothing
in the workspace turns on `testing`, so the fake's tests are unit tests in the crate, not doctests or `tests/` files.

## Gate and version

- The existing `fmt`, `clippy` and `workspace-tests` cover the crate. `scripts/ci/feature-builds.sh` gets
  `cargo check -p cicada-containers` with and without `testing`. `scripts/ci/dependency-trees.sh` checks that no
  library crate (`cicada`, `cicada-test`, `cicada-axum`, `cicada-client`, `cicada-otel`, `cicada-actors`,
  `cicada-app`) has `cicada-containers` in its tree (ADR 0056). No Docker check here: that is `container-docker`.
- Additive: patch 0.20.7. No guarantee, wire format, schema or command line changes.

## Docs it breaks

None: no doctest, link, guarantee row or skill route changes. For the docs session: a row for
`crates/cicada-containers` in the AGENTS.md map; the crate as optional in `README.md#layers` and
`docs/how-to/install.md#choose-the-crates`; a row for it, with ADR 0056, in the crate table of `docs/architecture.md`.
The skill's resolver (`PACKAGES` in `cicada_source.py`) does not know the crate yet: a project that depends on it also
depends on `cicada`, so `container-actor` adds it with its skill route.
