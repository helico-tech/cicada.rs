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

This is the first of two changes. It adds the runtime layer, which needs nothing from Cicada. `container-actor`
builds the ready-made container actor on top of it.

## What Changes

- A new crate `cicada-containers` in this workspace with:
  - a `ContainerSpec`: name, image, command, working directory, environment, secret environment (never on a command
    line), labels, user, CPU and memory limits, mounts and networks;
  - a `ContainerStatus`: missing, created, running, or exited with a code or a reason;
  - a `ContainerRuntime` trait. Every call is safe to repeat: create finds an existing container by its name, and
    removing one that is gone is not an error. It has create, start, inspect, the tail of the logs, stop, remove and
    list by label;
  - a Docker backend behind a feature, and a fake behind the feature `testing` for consumers' tests.
- A Docker test of the backend, run when Docker is present, like the E2E suite.

To decide in the design:

- **Backend:** `bollard` (typed errors, no output parsing; factory uses it) or the `docker` CLI (works with any
  Docker context and with Podman's CLI; brood uses it). Or both behind features.
- **Streaming output:** brood reads a container's output as it arrives (`start_attached`). Leave it to the consumer
  in this change, or add it as an optional part of the trait.
- **`copy_in`** (brood writes files into a container before it starts): in the trait, or left to mounts.
- **Networks and volumes:** create them through the trait, or only name existing ones in the spec. Factory makes a
  private network per step.
- **Errors:** one error type that says whether a retry can help, so the actor in `container-actor` can count errors
  in a row.
- Whether the crate's name and place need an ADR next to ADR 0002 (the crate stack), and whether a crate with
  Docker in it fits ADR 0011 (library only).

## Out of scope

The container actor (`container-actor`). Kubernetes, Podman's API or other runtimes beyond the one backend; the trait
leaves room for them. Image builds and registry login. Moving factory and brood onto the crate: each project does that
in its own repo. Any change to the existing crates.

## Order

After `dependency-advisories`, before `container-actor`.

## Impact

A new crate and its workspace entry; a new dependency (`bollard` or none) if the design picks one. Additive, so a
patch release. The gate gets a Docker check of the backend, or adds it to the E2E kind. An ADR if the design says so.
The docs handoff carries a short section for a guide and a skill route for the new crate; nothing that exists breaks.
