# Proposal

## Why

The two non-Rust worker ports (cicada-js, TypeScript; cicada-kt, Kotlin) track cicada.rs releases one to one and
implement the worker over HTTP (ADRs 0037–0040, 0042). They must match Rust exactly on contract hashes and on the
lease wire format, and today they mine Rust tests and snapshots for test data. The owner agreed to publish that data
with each release (decisions 32 and 33).

One rule is defined only by a library's behaviour. `canonical_json` (`crates/cicada/src/contract.rs`) prints numbers
through serde_json 1.0.151 with `preserve_order` and `float_roundtrip` (workspace `Cargo.toml`), which formats floats
with zmij 1.0.23 (`Cargo.lock`): fixed notation for decimal exponents -5 to 15, otherwise an exponent with a sign.
Reproduced with those pins: `1.0` → `1.0`, `1e16` → `1e+16`, `1e21` → `1e+21`, `0.00001` → `0.00001`, `0.000001` →
`1e-6`, `-0.0` → `-0.0`, `18446744073709551616` → `1.8446744073709552e+19`, `1.7976931348623157e308` →
`1.7976931348623157e+308`. `JSON.parse`/`JSON.stringify` give other text (`1` for `1.0`), so a port's hash differs
unless it reproduces this format. Parity on numbers is an accident of two dependency versions, not a documented rule.

That also makes every contract hash with a float in its schema depend on the serde_json and zmij versions in
`Cargo.lock`. A routine dependency update could change those hashes silently and refuse the registration of existing
Rust and non-Rust workers, with no changelog line saying so. The ports now re-check whenever `Cargo.lock` moves either
crate.

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
  could change the hash of existing contracts. The design decides whether the format stays tied to serde_json and zmij
  or is produced by Cicada's own code, so that a dependency update cannot change it.
- **A Rust test pins the number vectors and the hashes of the fixture contracts**, so a dependency update that changes
  the format fails before release instead of changing hashes silently.
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
