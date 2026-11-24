# Proposal

## Why

Factory (cicada-code-factory) makes empty folders with a mode before a run (its output and progress folders at 0777,
parents at 0755), so a program that does not run as root can write (Factory's review of the container designs,
2026-11-24). The runtime can write files into a created container (`copy_in`), but not make folders. The container
actor needs this to put a job's inputs in (`container-inputs`). Split out of `container-inputs` by its design review.

## What Changes

- `CopyDir { path, mode }` and `ContainerRuntime::make_dirs(name, dirs)`, with a default that fails with `Other`, so
  no implementation outside this repo breaks.
- It makes only what is missing, in a `Created` container: the named folders with their modes, missing parents with
  0755. An existing folder, whoever owns it, is left as it is; a file or link where a folder was asked is `Conflict`.
- The fake and the Docker backend implement it; the Docker backend checks what exists with Docker's archive stat.

## Out of scope

The actor (`container-inputs`). Changing an existing folder's mode or owner. `copy_in` of a file over a folder.

## Order

After `container-copy-out`, before `container-inputs`.

## Impact

The trait (an added method with a default), the fake, the Docker backend and its E2E class. A patch, released with
the rest of the container line.
