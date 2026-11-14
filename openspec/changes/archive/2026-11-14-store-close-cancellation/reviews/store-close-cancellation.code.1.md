---
id: store-close-cancellation.code.1
kind: code
change: store-close-cancellation
round: 1
base: e5f770ab77c65fe0d423a5843dd065ebcb431463
head: dc6fea2aa420f13e114600f70fcb7ffdfba542f2
requested: 2026-11-14T11:52:33+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done store-close-cancellation.code.1 <file>`.

## Note from the requester

Version: 0.17.4

Owner rule for this build (decision 24a): README and hand-written docs belong to the docs session; user-facing prose is in openspec/changes/store-close-cancellation/docs-handoff.md. Review the handoff instead of guide edits.

Builder's report:

Tasks (all done):
1. SQLite tests. On the old code 3 of the 6 new tests failed: a later or concurrent close returned at once and left the writer and the 4 idle readers open.
2. SQLite fix: every call runs the stop steps before its first wait, then callers take turns at a lock that remembers whether cleanup finished. Idle readers closed one at a time. A returning reader checks the closed flag under the pool's lock.
3. PostgreSQL tests. On the old code the borrowed-connection test failed at "the closed flag alone does not finish a close".
4. PostgreSQL fix: the listener's stop is split into request_stop and take_task; the unjoined listener task is kept between callers; a test replaces the listener task with a controlled one to prove it.
5. Doc comments on Store::close and Backend::close, changelog, version 0.17.4, the skill's cicada-version, docs/openapi.json regenerated. Architecture, operations and core-guide prose in docs-handoff.md.
6. Finishing checks.

Checks: cargo test --workspace with CICADA_PG_URL 2721 passed, 0 failed (incl. both SQL conformance suites); the new close tests passed 5 runs in a row; fmt --check, clippy -D warnings, cargo check -p cicada with no feature/sqlite/postgres, example doctests, docs and skill tests, openspec validate --strict: pass. Not run: Docker E2E.

Departures: none from the design. The builder flags two weaker tests:
- the test cancelling a close while a connection is being closed expects the first poll to stay pending; that depends on the driver's threads (a timing assumption, a miss very unlikely);
- the test of a reader returning while close drains the pool (20 rounds) can only catch the old bug by chance; on the new code it never fails.
Please judge whether these meet the repo's test rules (no sleep-based race tests; controlled barriers / fault hooks).

New concepts: resumable close (counts from its first poll; a never-polled close does nothing; if a caller stops waiting the store stays closed and the next close() finishes cleanup; concurrent callers return together). No time bound: PostgreSQL waits for connections operations still hold, SQLite for its writer but not for reads in flight; nothing finishes a cancelled close in the background.
Breaking: none.
