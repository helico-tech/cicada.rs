# 0007. Container handles

Date: 2026-10-11

## Status

Superseded by 0011

## Context

Services want actors that look after things running outside the Cicada runtime, starting with Docker containers: run
one, see its status, read its logs, stop and remove it. This is a Rust-only addition; the Kotlin implementation has no
counterpart, and the contract (guarantees, schema, HTTP API) does not change.

An actor exists only while it handles one message (architecture: Scope), a turn may run again when its node dies (G4),
and a container lives for hours on a Docker daemon of its own. So an actor cannot hold a running container; it can only
keep a durable record of one and act on it.

## Decision

A new crate, `cicada-docker`, provides a **container handle**: the ref `ContainerHandle` (`actor://containers/{name}`)
and the actor type `ContainerActor`, one instance per container.

- **Docker is the truth.** Every turn inspects the container in Docker before it acts; the state is only the last
  observation. The container's name is a fixed prefix plus the handle's name, so Docker's unique names turn "create"
  into "find or create", and a turn that runs again never makes a second container. Labels `cicada.uri` and
  `cicada.spec-hash` mark the container; a container with that name but another `cicada.uri` is not touched (the
  command fails permanently).
- **Commands, not a self-healing reconciler.** `Apply { spec }` makes the container run with that spec (it replaces a
  container whose spec hash differs), `Start`, `Stop` and `Remove` (removes the container, then the handle) act once,
  and `Check` only observes. An exited container stays exited, and a container removed outside Cicada is reported as
  `missing`; neither is recreated until the next `Apply` or `Start`. Restarts are Docker's job, through the restart
  policy in the spec. This keeps behaviour predictable (no restart loops driven by the poll) and fits run-to-completion
  containers.
- **Status by polling.** Each turn re-arms one `Check` self-send (cancelling the previous one), at an interval set on
  the actor type. A `Check` that cannot reach Docker records the error and re-arms, so polling never stops while the
  handle exists. A Docker events watcher is a later addition, not a replacement.
- **Logs bypass the store.** `container_logs(&containers)` is an axum `Router` with `GET /containers/{name}/logs` that
  streams the container's output straight from Docker (`tail`, `follow`, `since`, `timestamps`). The state keeps only
  the last lines at exit, for the failure reason.
- **One Docker endpoint.** Every node that hosts `ContainerActor` must reach the same Docker daemon (local socket for a
  single node, TCP/TLS for several), because any of them may run the next turn after a failover.
- **Engine seam.** The actor talks to Docker through the `Engine` trait; `DockerEngine` implements it with `bollard`,
  the async Docker client for tokio. Tests use a fake engine on virtual time; tests against a real daemon run when
  `CICADA_DOCKER=1`.
- Pulling an image can take minutes: the type's maximum turn duration is 10 minutes; lease and heartbeat cover it.

## Consequences

- `bollard` stays out of the core; services that do not run containers do not compile it.
- At-least-once still applies to Docker calls: a turn that runs again repeats them, and each is written to be harmless
  when repeated (find by name, start if not running, stop if running, remove if present).
- A turn of the old lease holder can still call Docker after a failover (epoch fencing guards only the store). The
  fixed name and the inspect-first rule keep this to a repeated, harmless call.
- Polling costs one turn per handle per interval; thousands of handles need the events watcher or a longer interval.
- Several Docker hosts need the host in the handle and its spec; not built now.
