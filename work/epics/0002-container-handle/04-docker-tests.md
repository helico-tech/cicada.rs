---
status: todo
created: 2026-10-11
---

# 04. Tests against a real Docker daemon

## Goal

Prove the handle and the logs router against a real daemon, locally and in CI.

## Acceptance criteria

- `CICADA_DOCKER=1 cargo test -p cicada-docker` runs a small image through apply, check, logs, stop, exit code, re-apply with another spec and remove; skipped when unset.
- Test containers use a prefix of their own and are cleaned up even when a test fails.
- CI sets `CICADA_DOCKER=1`.

## Plan

## Progress log

## Follow-ups
