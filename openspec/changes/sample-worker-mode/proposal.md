# Proposal

## Why

The sample service registers an `on_start` hook (`sample-service/src/app.rs`, about line 64) for its singletons and
the watch of its dead letters, so `worker --server` refuses it (ADR 0042 rejects `on_start` hooks in HTTP mode). Both
non-Rust worker ports (cicada-js, cicada-kt) therefore build their own small Rust crate to run a Rust HTTP worker next
to theirs in their E2E tests. The owner agreed at low priority (decision 31). The independent heartbeat-watch
catalog addition (decision 34) is split into the following `sample-heartbeat-watch` change after Opus 5.5 review.

## What Changes

- **A stock Rust HTTP worker** for interop tests: a second binary, `cicada-sample-executor`, shares the sample
  catalog and App lifecycle but has no `on_start` bootstrap. Its `worker --server` runs the sample's types over HTTP.
- Record actual executor identity through `Turn::node_id()` from `pinned-actors`, so App's `--node-id` override works
  for sample actors locally and over HTTP. Keep the existing state/message shapes and contract hashes.

Decided under the owner's delegation (2026-11-16; Opus 5.5 reviews): keep the existing sample binary and image server
entry point, ship the second binary in that image, and keep all target, token, type and shutdown policy in App.
Bootstrap remains explicit work for a store-connected host or API caller. Ready-made catalog additions are separate.

## Out of scope

Allowing `on_start` hooks in HTTP mode (ADR 0042 stays). The ports' own E2E suites. Conformance fixtures
(`conformance-fixtures`).

## Order

After `node-down-alert`. Before `sample-heartbeat-watch`.

## Impact

`sample-service/`, its new src/bin/cicada-sample-executor.rs binary, the sample image, the E2E suite, release
preparation and a docs handoff. No core/App behaviour, wire format, existing contract hash, store schema or guarantee
changes. Sample constructors retain legacy node-id arguments for source compatibility, but turn identity is
authoritative and those arguments no longer label records. No new ADR: this composes ADR 0042.
