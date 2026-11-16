# Proposal

## Why

The sample service registers an `on_start` hook (`sample-service/src/app.rs`, about line 64) for its singletons and
the watch of its dead letters, so `worker --server` refuses it (ADR 0042 rejects `on_start` hooks in HTTP mode). Both
non-Rust worker ports (cicada-js, cicada-kt) therefore build their own small Rust crate to run a Rust HTTP worker next
to theirs in their E2E tests. The sample service also hosts every ready-made actor except the heartbeat watch, which
0.18.10 added. The owner agreed to both, at low priority (decisions 31 and 34).

## What Changes

- **A stock Rust HTTP worker** for interop tests: a worker mode of the sample service without the `on_start` hook, or
  a small remote-worker example binary in the sample image. The design picks one; either lets `worker --server` run
  the sample's types against a node over HTTP.
- **The sample service hosts the heartbeat watch**, like the other ready-made actors, with what its E2E or smoke
  checks need.

## Out of scope

Allowing `on_start` hooks in HTTP mode (ADR 0042 stays). The ports' own E2E suites. Conformance fixtures
(`conformance-fixtures`).

## Order

After `node-down-alert`. No following change.

## Impact

`sample-service/`, possibly an example binary and the E2E image, the E2E suite, the changelog and a docs handoff. No
library behaviour, wire format, contract hash, store schema or guarantee changes.
