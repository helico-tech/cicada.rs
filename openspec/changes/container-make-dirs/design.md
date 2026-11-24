# Design

A host makes empty folders with a mode in a created container, so a program that does not run as root can write
there (Factory: output and progress folders at 0777, parents at 0755). Split out of `container-inputs` by its design
review, which then uses it from the actor.

```rust
pub struct CopyDir { pub path: String, pub mode: u32 }   // non_exhaustive, CopyDir::new(path): mode 0o755
// ContainerRuntime, next to copy_in():
async fn make_dirs(&self, name: &str, dirs: &[CopyDir]) -> Result<(), ContainerError> { /* default: Other */ }
```

## Decisions

1. **A new method; `copy_in` stays.** `copy_in(name, &[CopyFile])` is a public trait method: a new signature breaks
   every implementor and caller at a patch, which Cargo treats as compatible. `make_dirs` has a default that fails
   with `Other` ("not supported by this runtime"), as `container-logs` decided for added methods. Not taken: a
   changed `copy_in` (a break); a folder kind on `CopyFile` (`container-copy-out`: `copy_out` returns only files).
2. **It makes only what is missing; nothing that exists is touched.** For each named folder: an existing folder,
   whoever owns it and whatever its mode, is left exactly as it is, and so is a link whose target is a folder
   (decision 5); a file, any other link or anything else at that path is `Conflict` ("a file, not a folder": the
   program would fail later). A missing folder is made with its mode, and each missing parent with 0755; an existing
   parent is never touched, and one that is not a folder (or a link to one) is `Conflict`. A folder named in the list
   gets its own mode also when it is a parent of another named folder: a named mode beats the implied 0755, in any
   order. The same path named twice with different modes is `InvalidSpec`; twice with the same mode, it is made once
   (design review, blocking 3).
   The archive lists only the folders it makes, so `/tmp/out` leaves `/tmp` at 1777 (design review, blocking 2 and
   the note on parents). Every check is done before any write. Not taken: setting the mode of an existing folder (a
   Docker folder entry also makes it root's, so `/home/app` of user 1000 would become unwritable for its program);
   skipping a file at the path silently (the program finds no folder and fails later, far from the cause).
3. **The rest of `copy_in`'s rules.** The container must be `Created` (`Conflict` for a missing, running or ended
   one). The shared path rule of `container-consistency` applies: `InvalidSpec` for a bad path, a volume or bind
   mount target, a tmpfs path, and with a read-only root a path on no mount. A mode above 0o7777 is `InvalidSpec`.
   A repeat makes nothing, as everything exists then: the same end state.
4. **Docker checks what exists with its archive stat.** bollard 0.21 has `get_container_archive_info` (Docker's
   `HEAD /containers/{id}/archive`), which answers a path's mode and link target, or 404. For each named folder the
   backend asks for the path; on 404 it asks for each parent, upward, until one exists. The folder bit of the mode
   (Go's `ModeDir`) tells a folder; a link follows decision 5; any other type is `Conflict`. A stat error other than
   404 on a named path (Docker likely answers a path below a file with a 500) also starts the upward walk: a parent
   that is a file is `Conflict`; when none is, the original error maps by `kind_of` (design review, blocking 2). A 404
   whose container is gone (a second inspect) is `Conflict`. The missing folders then go up as a tar of folder
   entries, uid and gid 0, parents first, through the uploads of `copy_in` (one per mount with a read-only root).
   Not taken: Docker's
   `noOverwriteDirNonDir` (it refuses a file over a folder, but would still chown an existing folder); a tar of every
   level (resets the modes of existing parents, as the review found); running `mkdir` in the container (it has no
   process before start).
5. **Links are followed to a folder.** Docker's stat is an lstat: a link answers a link mode and its target, never
   the folder bit, and Debian-based images such as `postgres:18` have `/var/run -> /run` (design review, blocking 1).
   When the stat of a named path or a parent shows a link, the backend stats the target, resolved against the
   link's folder when relative and cleaned lexically (`..` stops at `/`), up to 8 hops. A target that is a folder
   counts as an existing folder, left as it is; anything else, a missing target included, is `Conflict`, and so is a
   ninth hop. The folders to make are named in the tar by their resolved path (`/var/run/app` goes up as `run/app`),
   so the upload never passes through a link, and the shared path rule and the mount grouping apply to the resolved
   path too. Not taken: refusing links (`/var/run/app` would be `Conflict` on Docker); letting Docker's extractor
   follow the link (its result would depend on how the extractor resolves links, and the mount grouping would use
   the wrong path).
6. **The fake follows the same rule.** It keeps made folders with their modes next to its files. A folder exists
   when it was made, or when it is `/` or a parent of a held file or folder; a held file at a named path or a parent
   is `Conflict`. The overlap rules of decision 2 hold. The fake has no links, so decision 5 is tested on Docker
   only. It records `Call::MakeDirs` and scripts `Method::MakeDirs`.

## Limits

The rustdoc of `make_dirs` names each one; the handoff carries them to the guide.

- The mode of a volume's own root folder cannot be set: the path rule refuses a mount target. Make the folder inside
  the volume, or set the mode in the image.
- An existing folder keeps its mode and owner, also when the caller asked for another mode.
- Something made by another caller between the stat and the upload is not seen; the container has no process yet,
  so only another API caller can race.
- Docker's answer for a path below a file (404 or another error) is not verified; the E2E pins it, and the backend
  maps it to `Conflict` through the upward walk either way (decision 4).

## Spec deltas

`container-runtime`: *Folders are made only where missing, before start* is added. `container-docker`: *Folders reach
Docker without touching what exists* is added. No guarantee, wire format, schema or actor contract changes, so no ADR.

## Tests

Unit tests of the fake (a made folder and its parents, a repeat, an existing folder keeps its mode, a file at the
path and at a parent is `Conflict`, a named parent keeps its mode in either order, one path twice with the same and
with different modes, a mode above 0o7777, the path rule, `Conflict` after start, a scripted failure), and of the
map: the tar holds only the missing folders, parents first, with their modes, under resolved paths; link targets
resolve (absolute, relative, `..`, a ninth hop). The E2E class `e2e/tests/suite/container_docker.rs` adds: `/out` at
0777 written by a program running as user 1000; `/tmp/out` leaves `/tmp` at 1777; `/run/postgresql` of the
`postgres:18` image keeps its owner and mode; `/var/run/app` makes `/run/app` through the image's link; a file at the
path and at a parent is `Conflict` (this pins Docker's answer for a path below a file); a read-only root with a
volume at `/work`, where `/work/cache` is made and `/etc/x` is `InvalidSpec`; `make_dirs` after start is `Conflict`.

## Docs it breaks

None: no README or guide block, link, guarantee row or skill route names the container runtime. `docs-handoff.md`
carries `CopyDir`, `make_dirs`, its rules and the limits for the container guide.

## Version

0.20.9, with the rest of the container line; released when the line, through `container-limits-and-kinds`, has
landed.
