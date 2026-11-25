---
id: watch-absent-target.code.1
kind: code
change: watch-absent-target
round: 1
base: 72e11e87d9c46555e4c9a71bbacb8f2cb60f0fea
head: 6305fcfb1ad7c87b97dee99533f363eee967fa07
requested: 2026-11-25T15:02:45+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done watch-absent-target.code.1 <file>`.

## Note from the requester

Version: 0.22.0 (BREAKING)

Builder report (head 6305fcfb): tasks 1-5 done. Conformance cases in crates/cicada/src/tests/conformance/watch_existing.rs;
PostgreSQL locks the target row in the ordered lock set, a race case of 40 rounds (fails when the lock is removed);
wire: existing left out when false, a bad one refused with 400; fixtures/leases.json gained one valid, one accepted and
two refused bodies; openapi regenerated; a remote-worker test over HTTP; pool test in crates/cicada/tests/watch_existing.rs.
Checks: fmt, clippy -D warnings, openspec --strict clean; workspace with PostgreSQL 3489 passed; examples docs 11, skill
18, doctests 34; gate full and e2e pass for 6305fcfb (one E2E retry after an unexplained image build failure).
Departure: every store's commit also refuses existing on a type, on *, or without set (StoreError::Invalid).
Docs: handed off, nothing blocks. The branch started before docker-context landed: its CHANGELOG 0.22.0 section must
absorb main's 0.21.2 section at rebase.
