---
id: remote-turns-over-http.code.2
kind: code
change: remote-turns-over-http
round: 2
base: b14f151b0a642fd506963600040d694622b34389
head: b67268f2d3f7011b6e33634d1a573d67911fa4f3
previous: bb2d39509c9bfbb28f43a6d443815bee6ff52307
requested: 2026-11-13T22:08:52+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done remote-turns-over-http.code.2 <file>`.

## Note from the requester

Version: 0.16.16

Builder report (round 2: fix of round 1)

Both blocking findings, the guide wording note and one more item from the build loop (a flaky test that failed the v0.16.15 release run) are fixed in cc8b84c, 3febcf2 and b67268f on feat/remote-turns-over-http.

Finding 1 (positional arrays): every record in the lease bodies, requests and responses now decodes only from a JSON object; array forms are refused. Each record's derived serde code is generated with #[serde(remote = "Self")] and a small wrapper lets it see only a JSON map; this holds inside tagged enums and lists. State and payload stay free JSON. Retry and retention decoders are shared with other routes and unchanged; in lease bodies only, a policy must be an object with no array anywhere in it. Schemas and docs/openapi.json unchanged; the drift test passes. New tests: an API test refuses array forms at every level, policies and responses included; a route test checks each gets 400, the lease is still held, nothing is deleted or created, and a valid completion then still succeeds.

Finding 2 (virtual time): the worker deadline, renewal and fault tests run on a paused clock over real sockets. The worker's source wraps ActorClient and holds every call at a barrier; a blocking task is alive for the whole call, so tokio does not move the paused clock during a request — the clock only moves between requests. Assertions are exact: the turn is held 4 s on at least 5 renewals through the second server; the cancel arrives with the first renewal, about 1 s after the lease; after a 413 nothing more is sent until the 3 s lease expires, then the handler runs again; after an unknown renewal answer the turn still runs at 2.65 s and is abandoned by 2.75 s; the lost-answer and rejected-commit tests wait 5 s and check nothing more is sent. The SQLite and PostgreSQL two-server tests are framing smoke tests on real time with no timing checks. 20 runs with PostgreSQL and 4 concurrent runs all passed.

Guide wording: docs/guide/http.md and docs/operations.md say a body cut off in transit is Unavailable, and a body that arrived whole but is malformed or has an unknown result is Fatal.

Flaky test (asked by the build loop; it failed the v0.16.15 release run): stop_returns_while_non_yielding_work_runs_on_and_nothing_is_released_under_it now waits for the handler-start signal before stopping the worker; the same race in a_cancelled_completion_after_a_shutdown_stays_within_the_confirmed_lease is fixed the same way. Those are the only tests that use BlockerActor or stop on Running. What they check is unchanged.

Checks
- cargo fmt --all --check passes; cargo clippy --workspace --all-targets -- -D warnings clean.
- cargo test --workspace with PostgreSQL: 2671 passed, 0 failed (before the flaky-test commit).
- The last commit touches only crates/cicada/src/tests/runtime/remote_worker.rs; its 16 tests passed 5 runs in a row; clippy clean.
- Not run: the Docker E2E suite.
