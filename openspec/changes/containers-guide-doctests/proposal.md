# Proposal

## Why

The docs session wrote `docs/guide/containers.md` for `cicada-containers` (2026-11-25). Its Rust blocks should compile
like those of the other guides, but the page is not wired as doctests, so the docs session left it without Rust
blocks. It asked for the wiring, with the owner's yes, to add a job, a log follower and a test on the fake as compiled
blocks.

## What Changes

- `examples/build.rs` turns `docs/guide/containers.md` into rustdoc markdown like the other guides, and
  `examples/src/lib.rs` gets `ContainersGuideDoctests`.
- `cicada-containers` becomes a dev-dependency of `cicada-examples`, with the features `testing` and `docker` (so the
  connect lines compile as `no_run`); the dependency-tree check covers only the library crates.
- AGENTS.md's list of doctest structs gains the new one through the docs handoff (the docs session owns AGENTS.md).

## Lane

Lane: fast

## Out of scope

The page's prose and blocks (the docs session writes them once this lands).

## Order

After `container-logs-unreadable`, before `app-tasks`.

## Impact

`examples/` only: tests. No release of its own.
