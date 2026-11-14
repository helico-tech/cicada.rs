---
id: app-worker-over-http.code.2
kind: code
change: app-worker-over-http
round: 2
base: b9773a3f6f898d06fc4158093ceab986069e8d5f
head: 16cd411234a0db2ae8028e67c58b67eac79c7b3b
previous: 53bc7ab379c0e89156ed56b8d0064bf9804ed5da
requested: 2026-11-14T07:58:22+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done app-worker-over-http.code.2 <file>`.

## Note from the requester

Version: 0.16.17

Round 2: fixes both round 1 blockers, in commit 16cd411.

1. Credential redaction: Target's Debug now redacts `server` structurally with a new `redacted_url` in crates/cicada-app/src/config.rs (URL parse, password replaced by ***). A value that is not a URL with a host prints only its scheme (`https://***`) or `***`, which also covers `user:pw@host` (otherwise parsed as scheme `user`). Regression: passwords with ', ", backtick, / and # are hidden whole; a URL without a password is unchanged; two unparsable values reveal nothing. `hide_passwords` is unchanged (its existing callers have the same apostrophe gap; out of scope, raised to the owner).

2. Timing on virtual time: three new paused-time tests in crates/cicada-app/src/node.rs use the remote-turns-over-http barrier harness with the real `Target::worker_client` against a real socket:
   - token and header on every route incl. renewals; a 4.5 s hold on a 3 s lease renews exactly 4 times (1, 2, 3, 4 s);
   - a refused token fails with exit 1 after one try in zero virtual time, token not printed;
   - stop during a running turn: takes effect at once (no lease request in the next 5 s), still waiting at 5 s, ends exactly when the handler does at 10 s; completed, not released.
   tests/worker_http.rs has no timing claims left (no Duration, sleep or elapsed): token test uses a quick message; refused-token test checks the single try and the error; the stop test checks only the outcome. One added test: a stop with zero grace releases an unfinished handler's lease with the token, and the message goes back to pending.

Checks: fmt and clippy -D warnings clean; cargo test --workspace with PostgreSQL 2712 passed, 0 failed; the 3 virtual-time tests 10/10 runs, worker_http 8/8.
