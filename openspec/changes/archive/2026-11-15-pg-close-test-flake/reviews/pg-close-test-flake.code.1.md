---
id: pg-close-test-flake.code.1
kind: code
change: pg-close-test-flake
round: 1
base: efaf746817a24ad3aee3e3e7a400e422254ac973
head: 0f43e56d88042c7df56afacef7aaf956bbb3245e
requested: 2026-11-15T08:18:52+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done pg-close-test-flake.code.1 <file>`.

## Note from the requester

Version: 0.18.8 (test fix; no production code)

Builder's report:

Task 1 done: the PostgreSQL test in crates/cicada/src/tests/postgres/changes.rs still sets up the store on its own runtime, then polls and drops the cancelled close once on a fresh current-thread runtime and asserts zero live tasks there. Same name, every earlier check and both teardown legs kept; it now also asserts the listener is stopped. The SQLite twin is unchanged: its harness never reaches the spawning drop path, and sqlx-sqlite 0.9.0 runs its connection workers on OS threads. Task 2 done: version 0.18.8 in Cargo.toml, Cargo.lock, skill metadata and docs/openapi.json; changelog entry; docs-handoff.md. Task 3 done; results in the tasks.md progress log and the handoff.

Mutation evidence: a temporary tokio::spawn right after the closed flag, and separately a guard whose Drop spawns through Handle::try_current(), each failed the test (1 task alive, 0 expected); both reverted, and the diff against main touches no production code.

Checks: the PostgreSQL test 20 times in a row, 20 passed; both twins together 2 passed (executed, not skipped); --test docs 11, --test skill 8; status.py no problems; fmt --check, clippy -D warnings, openspec validate --strict clean; cargo test --workspace with PostgreSQL twice: 2905 passed, 0 failed each. Not run: Docker E2E (release check). Repeated passes support the fix but do not prove the test can never flake.

Departures: none. Docs: handed off, no blockers. Breaking: none.
