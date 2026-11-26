---
id: placement-resource-key.code.1
kind: code
change: placement-resource-key
round: 1
base: a6f6aa90a08486a3bd21f54e64c2bd6d506f5d6a
head: ac78837991efd2ed5e851f84ad1341dbcebbfe3e
requested: 2026-11-26T20:43:41+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-resource-key.code.1 <file>`.

## Note from the requester

Version: 0.25.0 (the unreleased batch)

Builder report (head ac788379, rebased onto a6f6aa90): tasks 1-4 done; task 5's checks passed except the gate (the lock
was held by the orchestrator's landing run for the whole wait; the landing gate on main covers it). RuntimeInfo::id from
/info, with_id; Capacity::resource, Offer::resource; the contested list with one alert per entry; review fixes from
placement-identity-lock in their own commit bd1e0e5b (a read-only identity folder loads without a lock and warns; a
repeated duplicate is logged again after a clean round), each test failing without its fix. Checks: workspace with
PostgreSQL 3719, clippy -D warnings, fmt, openspec --strict, examples doc 51 / docs 11; the E2E suite compiles but did not
run (it runs in the landing release). Choices: contested always written ([] when empty); the check timer stays armed while
a contest is listed; a new contest past maxNodes refused for good; a crate-private offer_of helper. Do not block on the
missing gate evidence.
