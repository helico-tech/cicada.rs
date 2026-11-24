# Proposal

## Why

Two projects on Cicada run Docker containers, and each wrote its own layer for it:

- **cicada-code-factory**: a `Runner` trait (`start`, `inspect`, `logs`, `stop`, `remove`, `sweep`) over `bollard`,
  with a fake for tests (`src/runner/`, about 600 lines).
- **brood**: a `ContainerRuntime` trait (`create`, `copy_in`, `start_attached`, `exit_status`, `remove`,
  `list_labelled`) over the `docker` CLI, with a fake for tests (`crates/brood-agent/src/container.rs`, about 800
  lines).

Both solve the same problems: a container found by a fixed name so a repeated call is safe, removing leftovers by
label, reading how a container ended, keeping secrets off the command line, and a fake that runs tests without
Docker. Each got these details right separately, and the next project would write them a third time.

This is the first of three changes. It adds the runtime layer: a trait, its types and a fake, which need nothing
from Cicada. `container-docker` adds the Docker backend, and `container-actor` builds the ready-made container actor
on top of both. ADR 0056 lets a layer that two consumers share live in an optional crate of this workspace.

## What Changes

- A new crate `cicada-containers` in this workspace with:
  - a `ContainerSpec`: name, image, entrypoint, command, working directory, environment, secret environment (never on
    a command line or in `Debug` output), labels, user, CPU and memory limits, mounts, networks, and hardening fields
    that are off by default (read-only root file system, no capabilities, no new privileges, tmpfs mounts, an init
    process, a process limit);
  - a `ContainerStatus`: missing, created, running, exited with a code, or failed with a reason;
  - a `ContainerRuntime` trait. Every call is safe to repeat: create finds an existing container by its name, start
    does not run an exited container again, and removing one that is gone is not an error. It has create, copy files
    in, start, inspect, the tail of the logs, stop, remove, list by label, ensure a network, ensure a volume, and a
    sweep that removes containers, networks and volumes by label;
  - one error type, `ContainerError`, whose kind says whether a retry can help;
  - a fake runtime behind the feature `testing` for consumers' tests.
- ADR 0056: shared layers for external resources in optional crates; it supersedes in part ADR 0011.

The choices the old "To decide" list named (backend, streaming, `copy_in`, networks and volumes, errors, the ADR) are
decided in `design.md`.

## Out of scope

The Docker backend, its image pull and its Docker test (`container-docker`). The container actor (`container-actor`).
Streaming a container's output. Kubernetes, Podman's API or other runtimes; the trait leaves room for them. Image
builds and registry login. Moving factory and brood onto the crate: each project does that in its own repo. Any change
to the existing crates.

## Order

After `listening-line-race`, before `container-docker`.

## Impact

A new crate and its workspace entry, with `async-trait` and `thiserror` as its only dependencies. The gate's feature
builds and dependency trees name the crate. ADR 0056 and its row. Additive, so a patch release. The docs handoff
carries the crate's place in the map, the layers and the install guide; nothing that exists breaks.
