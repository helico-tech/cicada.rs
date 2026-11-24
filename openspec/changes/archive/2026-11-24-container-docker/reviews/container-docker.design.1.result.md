Verdict: not yet

## Blocking

1. **`copy_in` on a read-only root cannot work as designed** (design.md, decision 7). Docker checks whether the
   *extraction directory* lies on a mount, not each tar entry. One tar uploaded at `/` is therefore always refused
   when `ReadonlyRootfs` is set, even if every file is on a volume. The refusal is 403, which the error table maps to
   `Other`. `container-runtime` decision 9 says a path on a mount works and any other path is `InvalidSpec`. Fix: before
   calling Docker, refuse a path outside every `mounts` target with `InvalidSpec`, then upload one tar per mount target,
   with paths relative to it. Add a scenario and an E2E assertion. Also decide how tmpfs counts. I believe Docker does
   not mount tmpfs for archive calls on a created container. A file copied there then lands in the root, which is
   refused when read-only and hidden by the tmpfs after start otherwise. Say that in the host docs.
2. **`create` has two readings** (decisions 4 and 5). Decision 4 says every container method inspects first. Decision 5
   lists: image inspect, pull, create. If the image comes first, a repeated create whose image is gone, or a second spec
   with an unknown tag, pulls or fails with `ImageNotFound`. That breaks "create SHALL return Ok when a container with
   the name exists". Fix: inspect the container by exact name first and return `Ok` if it is there.

## Notes

- The proposal says "a name in use is `Conflict`", but decision 5 makes a 409 on create `Ok`. Align the proposal.
- `start`: treat 304 (already started, a race after the inspect) as `Ok`, as `stop` does. `sweep`: say that a failed
  container removal other than `DaemonUnavailable` is a skip.
- Secret scrub: skip empty secret values, or replacing "" mangles every message.
- Use the existing workspace `futures = "0.3.34"` instead of adding `futures-util`.
- Size: five tasks are tight. Split the backend into two tasks (create and its mappings; then the rest), then the E2E
  class, then the gate, version and handoff.

## Questions for the owner

- `connect()` ignores Docker CLI contexts, but the E2E suite finds Docker through the CLI (OrbStack or Docker Desktop).
  Does `/var/run/docker.sock` exist on the gate machine, or should the class set `DOCKER_HOST`?
- Is it OK that every E2E run pulls `busybox` from Docker Hub (needs network, has rate limits)?

## Not checked

Which crates bollard adds to `Cargo.lock`; bollard's per-call timeout API; Docker's status codes (403 for a read-only
root, 304 for start), which I took from memory of the API docs; Windows.
