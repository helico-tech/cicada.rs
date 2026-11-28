# Design

An `internal` Docker network has no route outside. But Docker still gives the bridge an address on the host, so the
network's containers can reach every host service that listens on that address or on `0.0.0.0`. That bypasses
Factory's egress proxy. Docker 28.0.0 (API 1.48, moby#49262) added the bridge gateway mode `isolated`. It is allowed
only on an `internal` network, and it leaves the bridge without a host address. The owner agreed the picks below
(N1a–N4, 2026-11-28). The Docker facts come from Docker's release notes for 28.0.0 and its port publishing page.

## Decisions

1. **An `isolated: bool` on `NetworkSpec`** (N1a). It defaults to false; `NetworkSpec::new` stays as it is. Its doc
   says what it closes: no host address on the bridge, so neither the host nor its published ports can be reached.
   The struct is `#[non_exhaustive]`, so this does not break callers. Not taken: a map of free driver options. It is
   more general but easy to get wrong, it ties the trait to Docker, and a mismatch check would have to compare
   arbitrary keys. A host that needs another option can get it in a later change.
2. **`isolated` requires `internal`.** Docker refuses the mode on a network that is not `internal`. Both runtimes
   check this first and fail with `InvalidSpec` without changing anything. The Docker runtime does not call Docker
   for it. Not taken: setting `internal` implicitly. A spec that says `internal: false` would then silently get
   something else.
3. **Docker: both gateway modes.** An isolated network is created with the driver options
   `com.docker.network.bridge.gateway_mode_ipv4=isolated` and `com.docker.network.bridge.gateway_mode_ipv6=isolated`
   (N4). The IPv6 option keeps the network closed if IPv6 is enabled on it, now or by a daemon default. Without
   IPv6 it has no effect. A network that is not isolated gets no options, as today.
4. **Too old: refuse, never fail open** (N2a). The runtime agrees the API version once (`docker()`); bollard 0.21
   asks for up to 1.53. The floor of the whole runtime stays at 1.44 (Docker 25). Only `ensure_network` of an
   isolated spec requires 1.48. On an older agreed version it fails with `Unsupported` right after the `InvalidSpec`
   check, before it looks for an existing network or creates one. The message names both versions and Docker 28.
   The order matters: an old daemon keeps driver options it does not know and shows them on inspect, so a network
   made by hand or left over from a downgrade would otherwise pass decision 5 while its bridge still has a host
   address (design review 1). A retry cannot help, which is what `Unsupported` means. Not taken: raising the floor
   to 1.48 (it would break hosts that never ask for isolation), and inspecting the network after creating it
   (Docker reports the options it was given, so that proves nothing more than the version does).
5. **An existing network must match on both flags** (N3). The existing check compares `internal`. It also compares
   `isolated`, read from the inspected network's options: the network counts as isolated when both gateway modes
   are `isolated`. A difference fails with `Conflict`, and the message names the flag. Docker cannot change these
   in place, and a silent `Ok` would hand back an open network. The fake does the same with its stored spec.
6. **Pure parts in `docker/map.rs`.** Three pure functions: the check of a spec against the agreed version
   (`InvalidSpec`, then `Unsupported`), which `ensure_network` calls first; the create request built from the spec
   (with or without the options); and `(internal, isolated)` read from an inspected network. Their unit tests cover
   each case without a daemon. `mod.rs` keeps the calls.
7. **E2E against the real daemon.** The gate's E2E suite (`e2e/tests/suite/container_docker.rs`; the local daemon
   is Docker 29.4, API 1.54) gets a test:
   - The isolated network is created, repeats with `Ok`, and `docker network inspect` shows both options.
   - Each flag conflicts both ways.
   - Behaviour, with a control case. A listener container is started with the docker CLI on the host network (as
     the suite's `docker run` at line 742 already does; `Networking` gets no host variant). A container on a plain
     `internal` network connects to it on its network's gateway address. A container on the isolated network
     cannot connect to the first address of its subnet: Docker reports no gateway for an isolated network, which I
     checked on Docker 29.4.
   If the E2E host cannot run host networking, the builder reports this as a design question and does not drop the
   check.
8. **No ADR, no wire change.** `NetworkSpec` is not in any contract (the container actor does not create networks).
   ADR 0056 already allows new runtime options. A host's own `ContainerRuntime` written before this change ignores
   `isolated` and creates an open network. The trait's `ensure_network` doc (`runtime.rs`, "Only the `internal`
   flag…") says that an implementation must honour `isolated` or fail with `Unsupported`. The docs of `Conflict` and
   `Unsupported` in `error.rs` name the new cases. CHANGELOG: an Added line under unreleased that says the same
   about hosts' own runtimes.

## Docs this breaks

None: no doctest builds a `NetworkSpec`, and no guarantee row or skill route names it. The docs handoff asks for one
paragraph in `docs/guide/containers.md` near the `ensure_network` row: `isolated`, that it needs `internal` and
Docker 28, and what it closes; and an upgrade step: a network that already exists as `internal` without isolation
fails with `Conflict` once a host asks for `isolated`, so remove it first (Factory's profile networks).

## Out of scope

Firewall rules outside Docker, the egress proxy, and other driver options. Sysctls are covered by
`container-sysctls`.
