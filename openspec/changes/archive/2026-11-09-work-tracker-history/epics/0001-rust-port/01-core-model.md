---
status: done
created: 2026-10-07
---

# 01. Core model

## Goal

Core model: addressing, time, policies, store trait and model, contracts and schemas.

## Acceptance criteria

- `ActorAddress`, `UriTemplate`, percent-encoding and ref params match the Kotlin rules and tests.
- Retry and retention policies serialize to the Kotlin JSON and validate with the same messages.
- Store model, results and support helpers (envelope JSON, retention rules, cursors) exist.
- Contracts hash canonical JSON; message families split into per-type schemas.

## Plan

Port `actors-core` address, retry, retention, store, contract and serialization packages.

## Progress log

- 2026-10-07: done in the initial build.

## Follow-ups

- (none yet)
