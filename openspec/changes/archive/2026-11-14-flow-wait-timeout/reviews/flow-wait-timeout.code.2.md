---
id: flow-wait-timeout.code.2
kind: code
change: flow-wait-timeout
round: 2
base: c55a445e7aa0c82f724a0d4aa68cfb36c877ea4a
head: bf210a573b8645f6f6488f0e07ccb1a7d9721272
previous: 52db637b362205f4eb5bf551d61e92af04dcadac
requested: 2026-11-14T15:17:53+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-wait-timeout.code.2 <file>`.

## Note from the requester

Version: 0.18.2

Round 2: fixes the round 1 blocker in commit bf210a5 (test only, plus async-trait as a cicada-actors dev-dependency; it is already a workspace dependency).

New test `a_rejected_timeout_decision_commits_nothing_and_a_reply_or_a_later_decision_wins` (crates/cicada-actors/tests/flow.rs):
- The flow runs in a Worker behind a small TurnSource wrapper around the node's ActorSystem. When armed, it rejects the first completion whose new state closes a wait as timed out, answering CompleteTurnResult::Rejected with cause `commit`, not retryable (what a store rejection gives), keeps the rejected report, and passes every other call through.
- After the deadline's turn it asserts: the rejected report holds the prepared timedOut state and one Continue due now; the state JSON and version are unchanged and the wait is still open with the same deadline; nothing is pending and the prepared Continue id has no outcome (never sent); the timer's message ended failed with the rejection reason; no timeout reached flow code and no closure ran.
- Recovery, two cases: a reply from the target then commits Received and the flow gets the value; without a reply, a public Continue makes a second timeout decision, which commits.
- The panic test stays, relabelled as coverage of an aborted execution.

Checks: fmt --check and clippy -D warnings clean; cargo test --workspace with PostgreSQL 2815 passed, 0 failed.

Please check the round 1 finding is closed and the test adds no new problem.
