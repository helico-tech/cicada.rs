# Proposal

## Why

The 0.23.1 release failed its E2E suite (2026-11-26): `ContainerDockerTest::copy_out_keeps_within_its_limits` expects
that a skipped big file leaves the small files after it whole. `collect-timeouts` changed that on purpose (the default
`max_read` of 4 × `max_bytes` now stops the copy at a file that would go past it) and updated the unit tests, but the
E2E test was not run for that change.

## What Changes

- The E2E test gives that check a large `max_read`, so it still tests the byte limit, and adds a check that the default
  read limit stops the copy with one skipped entry for the rest.

## Lane

Lane: fast

Release: now

## Order

First in the line, before `placement-offers`.

## Impact

`e2e/tests/suite/container_docker.rs` only. It unblocks the 0.23.1 release.
