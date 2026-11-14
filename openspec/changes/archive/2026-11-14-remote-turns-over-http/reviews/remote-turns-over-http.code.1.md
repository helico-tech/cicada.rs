---
id: remote-turns-over-http.code.1
kind: code
change: remote-turns-over-http
round: 1
base: b14f151b0a642fd506963600040d694622b34389
head: bb2d39509c9bfbb28f43a6d443815bee6ff52307
requested: 2026-11-13T21:50:39+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done remote-turns-over-http.code.1 <file>`.

## Note from the requester

Version: 0.16.16

Builder report (round 1)

remote-turns-over-http is built on branch feat/remote-turns-over-http, version 0.16.16. All 6 tasks done; every finishing check passes. Nothing pushed or merged.

Tasks
- Addition asked by the build loop (note of the remote-worker-runtime review): BlockerActor fires a notify when its handler starts, and work_that_outlives_the_cancel_timeout_... stops the worker only after that signal. What the test checks is unchanged.
1. Wire views in cicada::api::leases: done.
2. The four /leases routes, the duplicate-key parser and OpenAPI: done.
3. TurnSource for ActorClient: done.
4. Worker integration and fault tests, plus example 15 (examples/src/remote_worker.rs): done.
5. Docs: done (ADR 0040 and its index row were already there).
6. Finishing checks: done.

Checks
- cargo fmt --all --check: passes.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2669 passed, 0 failed.
- Each feature of cicada and cicada-app alone: builds.
- openspec validate --strict: valid.
- OpenAPI regenerated at 0.16.16; docs and doctests pass.
- Not run: the Docker E2E suite.

Departures and open questions
- The worker integration tests (Worker -> HTTP -> two servers) use real time, not start_paused: a paused clock can jump ahead while a request is in flight on a real socket and fire timeouts by mistake. The route tests run on virtual time. Timeout tests pause the clock only after the server holds the request.
- #[schemars(required)] removes null from a field's schema, so required-but-nullable fields use a small Nullable<T> schema type; a test checks that the schema and the decoder agree.
- A completed answer whose cause does not match its outcome is treated as Fatal.
- cicada-examples now depends on cicada-client and reqwest.

New concepts
- /leases routes: four POST routes on the full router only. Answers like "lease lost", "contract mismatch" and "rejected" come back as 200 with a result tag, so a proxy's 404 or 409 cannot be mistaken for one.
- ActorClient as a turn source: a worker needs only a server's URL, no database. Each call sends one request and never retries; errors are judged by the HTTP status alone.
- turn_timeout: limits each lease, renew, release and complete call, including reading the body. Default 10 s; zero is refused.

Breaking changes: none.
