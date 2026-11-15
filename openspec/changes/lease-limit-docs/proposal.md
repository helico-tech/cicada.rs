# Proposal

## Why

The lease request's wire docs and the code disagree on its limits. The doc comments of the request body say "1 to 64
types" and "The most turns to lease, 1 to 256" (`crates/cicada/src/api/leases.rs`, about lines 165 and 168), and
`docs/openapi.json` repeats them (about lines 3596 and 3603). The code refuses more than 100 of either:
`MAX_LEASE_TYPES = 100` and `MAX_LEASE_BATCH = 100` in `crates/cicada/src/runtime/system.rs` (about lines 57 and 60),
checked in `check_lease_request`. ADR 0039 also says a worker accepts at most 100 hosted types. A worker written from
the docs gets `Invalid` for 101 to 256 turns. Both non-Rust worker ports (cicada-js, cicada-kt) found this and follow
the code for now. The owner agreed to fix it first (decision 35).

## What Changes

- The doc comments of the lease request state the limits the code checks: 1 to 100 types and 1 to 100 turns, named
  after the constants, so the regenerated `docs/openapi.json` carries them.
- A test fails when the documented limits and the constants disagree, so they cannot drift apart again.
- Check the other remote-turn bodies (renew, release, complete) for the same kind of mismatch and fix any found.

## Out of scope

Changing the limits themselves. Conformance fixtures for the ports (`conformance-fixtures`).

## Order

After `heartbeat-watch`. Before `node-details`.

## Impact

Doc comments in `crates/cicada/src/api/leases.rs`, the regenerated `docs/openapi.json`, one test, the changelog. No
behaviour, wire format, contract hash, store schema or guarantee changes.
