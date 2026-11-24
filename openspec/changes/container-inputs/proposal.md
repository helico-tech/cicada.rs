# Proposal

## Why

Factory (cicada-code-factory) wants to move its container runs onto `cicada-containers` 0.20.7 and drop its own
`crates/containers` with its `docker` CLI backend (request from the Factory session, 2026-11-24; reference: Factory's
`docs/design/containers.md` and `crates/containers/src/runtime.rs`). It would be the third consumer (ADR 0056). The container actor goes from `create` straight to `start` and never calls `copy_in`, so a host cannot put files
and secrets in as files, only as environment variables.

## What Changes

- `ContainerJob::files(&self, name, &Request) -> Result<Vec<CopyFile>, String>`, empty by default, built at each create
  like the spec, so secrets never reach the store.
- The actor calls `copy_in` between create and start, only while the container is `Created`; a replayed step skips
  it. An `Err` ends the container as a bad spec does.

## Out of scope

Copying files out (`container-copy-out`). Changing files after start.

## Order

After `container-copy-out`, before `container-limits-and-kinds`.

## Impact

The actor and its tests; `ContainerJob` gains a method with a default, so hosts compile unchanged. A patch.
