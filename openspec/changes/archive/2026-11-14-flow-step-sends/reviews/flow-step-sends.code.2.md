---
id: flow-step-sends.code.2
kind: code
change: flow-step-sends
round: 2
base: d600969c3fd455afa4e18852124d3ffdb863d87e
head: a679e5bf5a16413c290c812a6abe19d4a1e75b95
previous: e0c50cc0a3ef49fbb7a72072d1e420f9e1948304
requested: 2026-11-14T13:58:08+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-step-sends.code.2 <file>`.

## Note from the requester

Version: 0.18.0 (breaking)

Round 2: fixes the round 1 blocker in commit a679e5b.

- check_waits (crates/cicada-actors/src/flow.rs): the check for an earlier open wait now runs before non-wait entries are skipped, so a step, sleep, send or request after an open wait is refused, not only another wait. Message e.g. "step `after` of the journal follows the open wait `w`, which must be the last entry".
- Regressions: a step, a sleep, a send and a request (a send expecting a reply) after an open wait, each in a running main journal and in the cleanup journal of a cancelling run; all refused. The wait-after-wait cases still pass with the new message.
- Schemas unchanged: a cross-field check, not JSON Schema, so no contract or snapshot regenerated.

Checks: fmt --check and clippy -D warnings pass; cargo test --workspace with PostgreSQL 2754 passed, 0 failed (same count: the cases went into an existing test).

Please check the round 1 finding is closed and the fix adds no new problem.
