# Proposal

## Why

The code review of `container-docker` (2026-11-24, notes 2-4) found three places where the fake and the Docker backend
of `cicada-containers` disagree, or where a call is stricter than the trait's repeat rules ask:

- A `copy_in` path that is exactly a mount target (such as `/work`) is accepted by one and refused by the other.
- `sweep` counts a container that was already gone as removed, but leaves out a network or volume that was already
  gone.
- `list` fails with `Other` when one container reports no state, instead of leaving that container out.

A consumer's tests run on the fake, so every difference is a bug that shows only in production.

## What Changes

- The fake and the backend give the same answer for a path that is exactly a mount target: refused with
  `InvalidSpec` (a file cannot replace the mount point).
- `sweep` reports something that was already gone in the same way for containers, networks and volumes: left out of
  `removed`.
- `list` leaves out a container whose state cannot be read, as it already does for one that vanished.

## Out of scope

New methods or fields. Moving consumers onto the crate.

## Order

After `job-graph-retry`, before `container-logs`.

## Impact

`crates/cicada-containers` (the fake and the Docker backend) and their tests, including the E2E class. A behaviour
fix of two calls, so a patch.
