---
id: container-job-max-turn.code.1
kind: code
change: container-job-max-turn
round: 1
base: f892f34074be864e8312fe6bd5c39257a93e5779
head: 06a8e0ac76ce12fc1d4efb0dea00e66f300b7467
requested: 2026-11-25T16:15:48+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-job-max-turn.code.1 <file>`.

## Note from the requester

Version: 0.22.0 (the unreleased batch)

Builder report: tasks 1-4 done; rebased onto main f892f340 (head 06a8e0ac). ContainerJob::MAX_TURN (default None) used as
ContainerActor's Actor::MAX_TURN, rustdoc names the create step and stop_grace; three virtual-time tests in tests/actor.rs
with a runtime wrapping the fake whose first create sleeps (20 min step fits a 35 min limit on a 15 min default; a 2 min
step under a 1 min limit times out and the retry makes no second container; a zero limit refused); they fail when the
actor ignores the job's value. Checks before rebase: cicada-containers all features 169+42+9 doctests, workspace with
PostgreSQL 3460, docs 11, fmt, clippy -D warnings, openspec --strict; gate full pass of bbafaa70 (pre-rebase head).
After rebase: cicada-containers, docs tests, fmt, clippy pass; the landing gate runs on main after merge.
No departures. Docs: handed off, nothing blocks.
