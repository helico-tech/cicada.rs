---
status: done
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

`tests/docker.rs`, gated on `CICADA_DOCKER=1`, real time (a paused clock would fire bollard's timeouts while it
waits for the daemon). Prefix `cicada-test-<pid>-`; a `Drop` guard runs `docker rm -f`. CI sets `CICADA_DOCKER=1`.

## Progress log

- 2026-10-11: Two tests pass locally against Docker 29.4 (OrbStack) in 3.5 s: a whole life (apply, logs, start and
  stop twice, replace with another spec, exit code 4 and its last line, remove, 404 after) and `follow=true` streaming
  until the container exits. No containers left behind.

## Follow-ups
