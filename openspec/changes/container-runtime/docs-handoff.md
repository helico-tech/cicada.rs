# Docs handoff for container-runtime

Version: 0.20.7. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

A new optional crate, `crates/cicada-containers` (ADR 0056), for applications that run containers. It needs nothing
from Cicada and no other crate of the workspace depends on it (the gate's `dependency-trees` check). No backend yet:
the Docker backend comes with `container-docker`, the container actor with `container-actor`.

- `ContainerRuntime`, an `async_trait` usable as `Arc<dyn ContainerRuntime>`: `create` (may pull), `copy_in`, `start`,
  `inspect`, `logs`, `stop`, `remove` (forced), `list` (by `Label`), `ensure_network`, `ensure_volume`, `sweep` (by
  `Label`, best effort). Every call is safe to repeat: `create` keeps an existing container of the same name without
  comparing it, `start` on a running or ended container is `Ok` and does nothing (a container runs at most once),
  `stop` leaves a created container `Created`, and `stop` and `remove` of a missing one are `Ok`.
- `ContainerSpec::new(name, image)`, with public fields set by name; `secret_env` is shown in `Debug` by name only.
  The hardening fields (`read_only_root`, `drop_all_capabilities`, `no_new_privileges`, `tmpfs`, `init`,
  `pids_limit`) are off by default. With them: `Mount` and `MountKind`, `Networking` and `NetworkAttachment`, `Tmpfs`,
  `NetworkSpec`, `VolumeSpec`, `CopyFile`, `Label`.
- `ContainerStatus`: `Missing`, `Created`, `Running`, `Exited { code }`, `Failed { code, reason }`. A stopped
  container is `Exited`. `ContainerSummary` (from `list`), `Swept`, `Skipped` and `Resource` (from `sweep`).
- `ContainerError { kind, message }` with `ErrorKind` `DaemonUnavailable`, `Conflict`, `ImageNotFound`, `InvalidSpec`,
  `Other`; `is_retryable()` is true only for `DaemonUnavailable`.
- Feature `testing`: `FakeRuntime` (clones share state; `exit_with`, `set_logs`, `fail_next`, `fail_always`,
  `clear_failures`, `calls()`), with `Call` and `Method`. No timers, so it runs under paused time.

Proved by the unit tests in `crates/cicada-containers/src/{spec,error,fake}.rs` and the crate's doctest in
`src/lib.rs`. No worked example: nothing in the workspace turns on `testing`, and no backend exists yet.

## 1. The crate in the map

**Where.** `AGENTS.md`, `Map`: a row after `crates/cicada-actors`.
**Text.** `crates/cicada-containers` | The container runtime (ADR 0056): the `ContainerRuntime` trait, its types
(`ContainerSpec`, `ContainerStatus`, …) and `ContainerError`; `FakeRuntime` behind the feature `testing`; no other
crate depends on it.
**Caveats.** None.

## 2. The crate as an optional layer

**Where.** `README.md`, `Layers`.
**Text.** A row: "Run containers from an application (optional, outside the core)" | `cicada-containers` | the
crate's rustdoc until a guide exists (`container-actor` may bring one).
**Caveats.** No backend yet in 0.20.7: the trait and the fake only. Say so, or wait for `container-docker`.

## 3. Choosing the crate

**Where.** `docs/how-to/install.md`, `Choose the crates`: the table.
**Text.** "Run containers" | "Add `cicada-containers` from the same tag; `features = [\"testing\"]` in
dev-dependencies for `FakeRuntime`".
**Caveats.** Same as 2: no Docker backend before `container-docker`.

## 4. The crate table

**Where.** `docs/architecture.md`, `Crates`: a row after `cicada-actors`.
**Text.** `cicada-containers` | An optional layer for applications that run containers (ADR 0056), with no
dependency on `cicada`: the `ContainerRuntime` trait (every call safe to repeat; containers, networks and volumes by
name; a best-effort sweep by label), `ContainerSpec` with secret environment kept out of `Debug` and hardening off by
default, `ContainerStatus`, `ContainerError` with a retryable kind, and `FakeRuntime` behind `testing`. No library
crate depends on it; the gate checks the trees.
**Caveats.** None.

## Blocking

None. No doctest, link, guarantee row or skill route changes. The skill's resolver (`PACKAGES` in
`skills/cicada/scripts/cicada_source.py`) does not know the crate yet; `container-actor` adds it with its route.

## Status

All four sections are proposed text; no prose is written yet. Checks: see the review request's head (the build report
lists `cargo test --workspace`, the docs and skill tests and the gate).
