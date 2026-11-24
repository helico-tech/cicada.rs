# Proposal

## Why

Factory (cicada-code-factory) wants to move its container runs onto `cicada-containers` 0.20.7 and drop its own
`crates/containers` with its `docker` CLI backend (request from the Factory session, 2026-11-24; reference: Factory's
`docs/design/containers.md` and `crates/containers/src/runtime.rs`). It would be the third consumer (ADR 0056). The container actor goes from `create` straight to `start` and never calls `copy_in`, so a host cannot put files
and secrets in as files, only as environment variables.

## What Changes

- `ContainerJob::inputs(&self, name, &Request) -> Result<Inputs, String>`, where `Inputs { dirs: Vec<CopyDir>, files:
  Vec<CopyFile> }`, empty by default, built at each create like the spec, so secrets never reach the store.
- The actor makes the folders (`make_dirs`, from `container-make-dirs`) and writes the files (`copy_in`) between
  create and start, only while the container is `Created`; a replayed step skips it. An `Err` ends the container as
  a bad spec does.

Factory's review (2026-11-24) adds: inputs need folders too, not only files. Factory makes empty folders with a mode
(its output and progress folders at 0777, parents at 0755) so a program that does not run as root can write. The
folders are a separate `CopyDir { path, mode }`, not a kind on `CopyFile`, so `copy_out` keeps returning only files
(container-copy-out design review); `CopyDir` and `make_dirs` are their own change, `container-make-dirs` (this
change's design review). Secret files stay on the container's own file system until removal (the tmpfs rule of
`container-consistency`).

## Out of scope

Copying files out (`container-copy-out`). Changing files after start.

## Order

After `container-make-dirs`, before `container-limits-and-kinds`.

## Impact

The actor and its tests; `ContainerJob` gains a method with a default, so hosts compile unchanged. A patch.
