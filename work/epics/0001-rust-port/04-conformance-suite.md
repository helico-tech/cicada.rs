---
status: todo
created: 2026-10-07
---

# 04. Store conformance suite S1–S10 as a reusable crate, run against InMemoryStore

## Goal

Store conformance suite S1–S10 as a reusable crate, run against InMemoryStore.

## Acceptance criteria

- Every Kotlin conformance case (`StoreConformanceSuite`, chain, reschedule, retention, robustness, turn-cancel suites) has a Rust counterpart.
- `InMemoryStore` passes it.

## Plan

Port `actors-store-conformance` to a crate exposing `async fn` cases over a `StoreFixture` trait plus a macro that generates `#[tokio::test]`s per backend.

## Progress log

- (none yet)

## Follow-ups

- (none yet)
