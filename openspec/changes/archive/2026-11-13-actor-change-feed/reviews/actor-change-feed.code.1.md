---
id: actor-change-feed.code.1
kind: code
change: actor-change-feed
round: 1
base: 45f7fc4ad422394fbd9ec4ecf3fadc3c75393fba
head: 4af1aac5b0598b842d64e839606442e3fd515493
requested: 2026-11-13T16:45:59+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-change-feed.code.1 <file>`.

## Note from the requester

Version: 0.16.10

Builder report (round 1)

actor-change-feed is built, version 0.16.10, on branch feat/actor-change-feed in six commits. Nothing pushed, merged or tagged.

Tasks
1. Core types, ActorSystem::changes, the shared hub and memory publication: done.
2. SQLite publication after a confirmed commit, and settlement of a cancelled or failed commit before one Resync: done.
3. PostgreSQL @ payload in one pg_notify with the wake-ups, 54000 at commit mapped to Unavailable, listener goes live only after LISTEN: done.
4. Conformance cases on every store, plus the S2/S3 rows in docs/guarantees.md: done.
5. Docs, changelog, version and the regenerated docs/openapi.json: done.
6. Finishing checks: done.

Checks (PostgreSQL on port 55432)
- cargo test --workspace with PostgreSQL: 2431 passed, 0 failed.
- cargo fmt --all --check, cargo clippy --workspace --all-targets -- -D warnings and openspec validate --strict: clean.
- cargo check of cicada and cicada-app with each feature alone: clean.
- Not tested: a runtime shutting down while SQLite settles a commit. The PostgreSQL 54000 error comes from a test trigger, not a really full queue.
- Not run: Docker E2E.

Departures and open questions
- Wake-up streams now also wake once on the listener's first LISTEN, and they end on Store::close. Three existing PostgreSQL tests now expect that first wake-up.
- After any Resync, hints already queued behind it are dropped; the reread covers them, as the design says.
- change is a public module; the root re-exports the two types.
- futures is a new dev-dependency of cicada-examples, for the guide's doctest.
- Test-only hooks: commit_fault on SQLite, rerun_next_commit on PostgreSQL.

New concepts
- Change hint: Changed { address } only says "read this actor again". It carries no state or version; the read is the truth.
- Resync: hints may have been lost, so reread the whole prefix. Always the first item, and also after lag, a reconnect, or a transaction of more than 100 addresses or 7,998 bytes.

Breaking changes
None to the API. On PostgreSQL, a full notification queue at COMMIT is now Unavailable instead of Invalid, and a commit that only writes state now sends a notification. (Neither is on the README's list of breaking changes, so the version is a patch; reviewer: say so if you disagree.)
