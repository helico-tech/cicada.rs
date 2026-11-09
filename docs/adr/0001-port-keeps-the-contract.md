# 0001. The Rust port keeps Cicada's contract

Date: 2026-10-07

## Status

Superseded by 0008

## Context

Cicada exists as a Kotlin library (helico-tech/cicada). Services in Rust should be able to host durable actors with the
same semantics, and existing callers (the console, HTTP clients, other services) should not notice which
implementation serves them.

## Decision

The Rust port keeps, unchanged: the domain language, guarantees G1–G6, store requirements S1–S10, the fourteen HTTP
endpoints with their bodies, status codes and problem types, the JSON forms of envelopes, retry and retention
policies, and the SQL store schema. It changes only how definitions are written (idiomatic Rust instead of a Kotlin
DSL) and which libraries supply HTTP, storage and async execution.

## Consequences

- The Kotlin `ARCHITECTURE.md`, `README.md` and `TICKETS.md` stay the reference for behaviour; Rust tests port the
  Kotlin tests where they pin behaviour.
- Deviations must be recorded as ADRs. Known ones: JSON Schemas differ in detail (schemars instead of kotlinx-schema),
  so contract hashes differ between the implementations (ADR 0004).
