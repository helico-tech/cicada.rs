# Proposal

## Why

`container-runtime` adds the `ContainerRuntime` trait, its types and a fake, but nothing that talks to Docker. Both
consumers run Docker: cicada-code-factory over `bollard`, brood over the `docker` CLI. Without a backend in the crate,
each still writes the hard half itself: mapping a spec onto Docker's create call with its hardening, finding a
container by name, reading how it ended, pulling a missing image, and sweeping by label in the right order.

The owner chose `bollard` (C-BACKEND, 2026-11-24): typed errors and no output parsing, and factory already uses it.
`container-runtime` decided that `create` may pull a missing image (C-PULL); this change builds it.

## What Changes

- A Docker backend in `cicada-containers` behind a feature `docker`, over `bollard`. It implements every method of
  `ContainerRuntime` with the repeat rules of `container-runtime`: create finds an existing container by name, start
  leaves a running or exited container alone, stop and remove of a missing container are `Ok`, the network and volume
  calls find what exists, and the sweep removes containers, then networks, then volumes.
- `create` pulls the image when Docker does not have it, and fails with `ImageNotFound` when the registry does not.
- Secret environment goes to Docker in the create request's body, never on a command line, and never in a log line
  or an error message.
- Docker's errors map onto the kinds of `ContainerError`: no connection to the daemon is `DaemonUnavailable`, a
  name in use on create is the existing container (`Ok`, as the trait's repeat rule says), a rejected spec is
  `InvalidSpec`.
- A test of the backend against a real Docker daemon, run in the gate's E2E kind (`CICADA_E2E=1`), so a full run
  without Docker still passes.

The decisions are in [design.md](design.md).

## Out of scope

The container actor (`container-actor`). The `docker` CLI as a second backend, Podman, Kubernetes or other runtimes.
Streaming output. Image builds and registry login. Any change to the trait or the fake of `container-runtime`, unless
the backend shows a mistake in them; that goes back through the change loop.

## Order

After `container-runtime`, before `container-actor`.

## Impact

`crates/cicada-containers`: the `docker` feature, the backend and its test. New dependencies behind the feature:
`bollard`, and what it needs (likely `tar`, `futures`, `tokio`). The gate's feature builds check the crate with
`docker`, the dependency-trees check keeps `bollard` out of the default build, and the E2E kind runs the Docker test.
Additive, so a patch release. The docs handoff carries how a host turns the backend on; nothing that exists breaks.
