---
id: remote-worker-runtime.code.1
kind: code
change: remote-worker-runtime
round: 1
base: 1fd016b9b6161a23f73660fa66c7a3a47d597433
head: 23bc447e473e11824b78712b552b7a4b5e45ae90
requested: 2026-11-13T19:58:13+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done remote-worker-runtime.code.1 <file>`.

## Note from the requester

Version: 0.16.15

Builder report (round 1)

All six tasks of remote-worker-runtime (0.16.15) are built; every check passed, PostgreSQL included. Four commits on feat/remote-worker-runtime.

Tasks
1. Source API: TurnSource and SourceError, with ActorSystem and Arc<T> as sources.
2. Shared executor: the node and the worker run turns through one executor, differing only in where the lease lives (the node's store or the worker's source). The node keeps its old behaviour; all old tests pass.
3. Worker life cycle: Worker, its builder, config and error; polling and manual batches share the capacity; stop drains turns.
4. Parity and fault tests: the same scenario runs on a local node and on a worker; end state, outcomes and observer events match.
5. Docs and version.
6. Finishing checks.

Checks
- cargo test --workspace with PostgreSQL: 2617 passed, 0 failed (docs/openapi.json regenerated, version only).
- cargo fmt --all --check, cargo clippy --workspace --all-targets -- -D warnings, cargo check -p cicada with no feature and with each feature alone: clean.
- Docs: 54 guide doctests and 11 docs tests pass. openspec validate remote-worker-runtime --strict passes.
- Not run: the Docker E2E tests (this change adds no HTTP).

Beyond the design, and open questions
- The stricter deadline check is on for workers only: it refuses a renewal or a completion at or after the deadline. Turning it on for nodes would change node behaviour; the build loop has taken that to the owner as a possible later change (not for this review).
- A public Worker::builder doc example and no_run crate docs added.
- A TTL under 1 ms cannot be tested end to end (tokio's virtual timers round up to 1 ms); those heartbeat and grant rules are tested directly.

New concepts
- Worker: runs unchanged actors without a database; gets turns from a source, renews them, reports each result back through it.
- TurnSource: the trusted interface a worker talks to; an in-process ActorSystem is one; an HTTP client can be one later.
- Per-turn deadline: each lease follows the TTL its server granted; a reply after the deadline cannot bring the lease back.
- Current report and budget: the worker retries one result within one TTL; a rejection replaces that result for good; only an acknowledged completion ends the observers.

Breaking changes: none.
