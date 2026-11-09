---
status: done
type: debt
created: 2026-10-07
---

# 0500. The docker-compose PostgreSQL allows only 100 connections

## Problem

The `postgres` service in `docker-compose.yml` runs with PostgreSQL's default `max_connections` (100). The conformance
suite runs 102 cases in parallel and some cases open 16 peer stores, so the PostgreSQL run of the suite refused
connections ("sorry, too many clients already") and failed at random. The Kotlin suite runs its container with
`max_connections=500`.

`crates/cicada-store-postgres/tests/conformance.rs` now runs at most 4 cases at once and gives peers a pool of one
connection, which keeps it below 100 (about 14 s instead of 10 s). Story 15 (multi-node end-to-end tests) may hit the
same limit.

## How to reproduce

Set `PARALLEL_CASES` in `tests/conformance.rs` to 102 and run
`CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada-store-postgres --test conformance`;
`docker logs` of the container shows the refusals.

## Notes

Option: `command: ["postgres", "-c", "max_connections=500"]` on the compose service (needs a container restart), as in
Kotlin's `PostgresTestDatabase`. Then the semaphore in the conformance test can be raised or removed.
