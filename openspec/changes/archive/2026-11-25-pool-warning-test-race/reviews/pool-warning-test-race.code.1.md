---
id: pool-warning-test-race.code.1
kind: code
change: pool-warning-test-race
round: 1
base: 2a215b087cb1e604e4ccf5fdce87dc059059b7da
head: 7d9b65855c1aaa2e92cf4548745ca99d558f60fc
requested: 2026-11-25T11:17:00+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done pool-warning-test-race.code.1 <file>`.

## Note from the requester

Version: none

Builder report (head 7d9b6585): cause under ## Cause in proposal.md (the only tracing subscriber in the cicada test binary;
tracing-core caches call-site interest from the first thread that reaches the warning; other PostgreSQL runtime tests log
the same warning with pool 6 at concurrency 4 on threads without a subscriber). Fix: in test builds, start also records the
warning in a per-thread list read by the test-only take_pool_warnings(); tracing::warn! unchanged. Checks: the test 10/10,
lib suite with the gate's profile 10/10; fmt, clippy clean; cargo test --workspace 3424 passed (one rerun for an unrelated
timing test); gate full pass of 7d9b6585. Departures: removed the unused tracing-subscriber dev-dependency of cicada
(Cargo.lock); the test no longer checks the WARN level. Docs: none.
