# Proposal

## Why

The two non-Rust worker ports (cicada-js, TypeScript; cicada-kt, Kotlin) track cicada.rs releases one to one and
implement the worker over HTTP (ADRs 0037–0040, 0042). They must match Rust exactly on contract hashes and on the
lease wire format, and today they mine Rust tests and snapshots for test data. The owner agreed to publish that data
with each release (decisions 32 and 33).

One rule is defined only by a library's behaviour. `canonical_json` (`crates/cicada/src/contract.rs`) prints numbers
through serde_json with `float_roundtrip` (workspace `Cargo.toml`): a schema literal `1.0` stays `1.0` and `1e21`
prints `1e21`. `JSON.parse`/`JSON.stringify` give `1` and `1e+21`, so a port's hash differs unless it keeps the source
text. Parity on numbers is an accident of serde_json today, not a documented rule.

## What Changes

- **Fixtures in each release**, a language-neutral archive next to the skill archive (ADR 0041), generated from the
  Rust sources and checked by a test so they cannot drift:
  - canonical-JSON and contract-hash vectors, including Unicode key ordering, float and exponent literals, and
    description stripping;
  - request and response JSON samples for `/leases`, `/leases/renew`, `/leases/release` and `/leases/complete`, with
    i64 extremes and every tagged result (`leaseLost`, `contractMismatch`, `rejected`, …);
  - the HTTP status classification table of ADR 0040 as data.
- **The canonical number format becomes a written rule** in the contract spec and an ADR (or an addition to the ADR
  that defines contract hashes), proven by the vectors above. Limiting schema numbers to integers is not proposed: it
  could change the hash of existing contracts.
- The release driver (`scripts/gate.py release`, ADR 0046) packages and verifies the fixture archive as it does the
  skill archive.

## Out of scope

A port's own tests, a conformance runner for other languages, and any change to hashing or the wire format itself.
The lease limit docs (`lease-limit-docs`). A stock Rust HTTP worker (`sample-worker-mode`).

## Order

After `node-down-alert`. Before `sample-worker-mode`.

## Impact

A generator and test for the fixtures, the release driver's packaging, the contract spec and an ADR, the changelog
and a docs handoff for the ports' guide. No behaviour, wire format, contract hash, store schema or guarantee changes.
