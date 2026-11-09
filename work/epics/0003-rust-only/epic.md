---
status: done
created: 2026-11-09
---

# Epic 0003: Rust only

## Goal

Cicada for Rust stands alone: no contract with, and no dependency on, the Kotlin implementation.
Decision: [ADR 0008](../../../docs/adr/0008-rust-only.md).

## Scope

Remove what exists only for Kotlin, and make the code and docs describe this library on its own. Not in scope:
changing the HTTP API; rewriting ADRs or finished work items.

## Done when

No live code or doc mentions Kotlin apart from ADR 0008 and history, and `cargo test --workspace`, clippy, fmt and the
console tests pass.

## Stories

- [01 Cut the Kotlin contract](01-cut-kotlin.md) — done
- [02 One baseline schema](02-baseline-schema.md) — done
