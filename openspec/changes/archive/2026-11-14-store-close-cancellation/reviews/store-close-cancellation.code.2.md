---
id: store-close-cancellation.code.2
kind: code
change: store-close-cancellation
round: 2
base: e5f770ab77c65fe0d423a5843dd065ebcb431463
head: c080e678c6bd78c04621a7bf461f119e616446dc
previous: dc6fea2aa420f13e114600f70fcb7ffdfba542f2
requested: 2026-11-14T12:19:15+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done store-close-cancellation.code.2 <file>`.

## Note from the requester

Version: 0.17.4

Round 2: fixes all three round 1 findings in commit c080e67. Only test hooks changed (each behind #[cfg(test)]); release code is as in round 1.

1. Cancelled while closing a connection: the test stops close right after it takes a connection for its own close and cancels there, three times (after the writer, the first reader, the second reader), asserting exactly 4, 3 and 2 idle readers remain, then that a later close empties the pool. A close that takes the whole reader list at once fails it ("after 1: left 0, right 3"), checked by swapping that version in once.
2. Reader returning during the drain: the 20 chance rounds are replaced by two fixed-order tests: (a) the reader stops before the pool lock, close finishes, the reader goes on: the pool stays empty; with the old order (flag read before the lock) it fails every time (3/3, 1 connection put back). (b) the reader stops under the lock after reading "open", close sets the flag: the drain waits for the lock and takes the returned connection; the pool ends empty.
3. Spec scenarios:
   - Concurrent first polls (SQLite and PostgreSQL): one close stops right after setting the closed flag and a second close overtakes it; the second still runs every stop step and the whole cleanup. With the old "return if the flag is set" check, SQLite fails on the change hub and PostgreSQL on the pool still open.
   - Final drop and runtime shutdown (both): a close is cancelled with cleanup unfinished, then the last handle is dropped, or the runtime is shut down first. The cancelled close started no task (exact task count for SQLite, no more than before for PostgreSQL); nothing else holds the store and dropping it frees it; no wait for the driver to finish closing connections.

Checks: fmt --check and clippy -D warnings pass (also cicada with no features); cargo test --workspace with PostgreSQL 2726 passed, 0 failed; the 23 close tests passed 10 runs in a row. Not run: Docker E2E. The linker warns the test binary's unwind table exceeds 16 MB (warning only; not checked whether it predates this change).
