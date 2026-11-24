# Design

Three fixes from the `container-docker` code review (notes 2-4). Each makes the fake and the Docker backend give the
same answer, so a consumer's tests on the fake predict production. No new method, field or error kind.

## Decisions

1. **One path rule for `copy_in`, shared by both runtimes.** Today the fake counts a path that equals a mount
   target as on that mount (`fake.rs:220-223`, `path == target`), while the backend needs a path strictly below it
   (`docker/map.rs:264`, `m.parts.len() < parts.len()`). With a read-only root and a volume at `/work`, the fake
   writes `/work` and the backend refuses it; with a read-only bind at `/in`, the fake refuses `/in` and the backend
   uploads it at `/`, where Docker fails late as `Other`. The two also parse paths differently: the backend drops
   empty and `.` parts and refuses `/` (`map.rs:200-215`); the fake keeps them and accepts `/` (`fake.rs:246-251`).
   Rule: a path is absolute, has no `..` part and names a file once empty and `.` parts are dropped; a path equal to
   the target of a volume or bind mount fails with `InvalidSpec` (a file cannot replace a mount point); otherwise the
   deepest mount strictly above it decides, as now. A tmpfs counts as a mount here: a path at or below a tmpfs
   target, whose deepest mount is that tmpfs, fails with `InvalidSpec` on any root (design review note 2). On a
   writable root the fake writes such a file today, and Docker uploads it before start, where the tmpfs mounted at
   start hides it: the file is lost without an error. The backend reads the tmpfs targets from the inspected
   `HostConfig.tmpfs`, as a created container has no tmpfs mount point yet. The fake stores the cleaned-up path, so
   `/work/./a` and `/work/a` are one file, as on Docker (note 5). `path_parts` and the mount choice move from
   `docker/map.rs` into a private module that is always compiled, and both runtimes call it, so they cannot drift again;
   `container-copy-out` and `container-inputs` can use it too. The rustdoc of `copy_in` names the new case. Not taken:
   the fake alone copies the backend's strict check (an exact target on a writable root still reaches Docker and fails
   late); accepting an exact target (Docker cannot write a file over a mount point); leaving the tmpfs case to
   `container-inputs` (it is the same rule in the same function, and a lost file is worse than a refusal).
2. **A sweep names only what it removed.** The backend counts a container that answers 404 or 409 as removed
   (`docker/mod.rs:383`), but leaves out a network or volume that answers 404 (`mod.rs:394`, `mod.rs:404`). Rule: the
   result names only what this sweep removed or skipped; something already gone, or a container another caller is
   removing, is in no list. Gone is 404 for every kind, and 409 for a container, which a forced remove gets only when
   another caller is removing it already (for a volume, 409 means in use and stays a skip). A pure
   `map::gone(resource, &error)` decides, with unit tests. The fake holds its lock for the whole sweep, so it never
   sees anything gone and does not change. Not taken: counting gone networks and volumes too (the result would claim
   work another caller did, and two racing sweeps would both name it); a skip with reason "gone" (nothing is left in
   place).
3. **`list` leaves out a container whose state Docker does not report.** The backend fails the whole `list` with
   `Other` when one container has no state or an empty status (`docker/mod.rs:308`, through `status` at
   `mod.rs:114-121`), but skips one removed since the listing (`mod.rs:304`). Rule: both are left out. `inspect`,
   `start`, `copy_in` and `stop` of that name still fail with `Other`: a call on one name must not invent a status.
   A pure `map::summary(&inspect_response)` returns `None` for it, with unit tests. The fake always knows a state and
   does not change. Not taken: report it as `Failed` (invents an end for a container that may run); keep `Other` (one
   odd container blocks every scan of a label).

## Spec deltas

`container-runtime`: *Files are copied only before start* (the path rule) and *Networks, volumes and sweeps are safe
to repeat* (what the result names) are modified. `container-docker`: *Files reach a read-only container only on a
mount* says "below" a target; *A list leaves out what it cannot report* is added. No guarantee, wire format, schema or
actor contract changes, so no ADR.

## Tests

Unit tests for the shared path rule, `gone` and `summary`; the fake's tests for the exact target, a tmpfs path on a
writable root, `/` and the cleaned-up path. The E2E class `e2e/tests/suite/container_docker.rs` adds the exact target,
with a read-only and a writable root, and a tmpfs path on a writable root. A gone item in a sweep and a container
without state cannot be staged against a real daemon; the unit tests cover them.

## Docs it breaks

None: no README or guide block, link, guarantee row or skill route names these calls (only ADRs 0056 and 0057 name
the crate). The rustdoc of `copy_in`, `list`, `sweep`, `Swept` and `DockerRuntime` is source and changes with the
code. `docs-handoff.md` carries the three rules for the container guide when the docs session writes it. Hosts have
used the crate since 0.20.7, so the changelog names the refused paths, the `Swept` lists and `list` under Fixed
(note 4).

## Version

0.20.9, the patch after the unreleased 0.20.8 batch. It is released when the whole container line, through
`container-limits-and-kinds`, has landed.
