---
id: actor-change-sse.code.1
kind: code
change: actor-change-sse
round: 1
base: 9d174a8a82b949a44887db76dd9f31f35b99cd97
head: e7135a3f7679e90ff0219e12f2cfd0d87ece1853
requested: 2026-11-13T17:13:07+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-change-sse.code.1 <file>`.

## Note from the requester

Version: 0.16.11

Builder report (round 1)

actor-change-sse is built as 0.16.11. All five tasks done, every check passes. Branch feat/actor-change-sse, 4 commits on top of main. Nothing pushed, merged or tagged.

Tasks
1. Route: done. GET /changes on both routers. New module crates/cicada-axum/src/changes.rs turns each hint into an event. 14 tests in crates/cicada-axum/tests/changes.rs, including a Rust copy of the guide's browser consumer.
2. OpenAPI: done. Private EventStream marker; docs/openapi.json regenerated. Old routes unchanged.
3. Shutdown tests: done. serve's last three steps (serve, stop the node, close the store) moved into one function, serve_and_close, so a test can call them; the order did not change. Checked that the guide's shutdown hangs if the store close is removed.
4. Docs: done. Crate docs, architecture, HTTP guide (with the JavaScript example), operations, changelog and version.
5. Finishing checks: done.

Checks
- cargo fmt --all --check: passed.
- cargo clippy --workspace --all-targets -- -D warnings: no warnings.
- cargo test --workspace with PostgreSQL: 2450 passed, 0 failed.
- Examples doctests and the docs links test: passed.
- openspec validate actor-change-sse --strict: valid.
- The JavaScript example is only syntax-checked with node --check; not run in a browser or behind a proxy.
- Not run: Docker E2E.

Departures and open points
- No departure from the design.
- The "400 before the stream starts" case has no test: no query the existing parser refuses was found (it decodes bad bytes leniently).
- That a dropped body frees its subscription is not checked directly; the count is private to the core.

New concepts
- GET /changes: a Server-Sent Events stream. changed means "read this actor again", resync means "list the prefix again". The first event is always a resync; no replay.
- Keep-alive comments: a ": keep-alive" line after 15 s of silence, to keep proxies from closing the connection; it does not mean the stream is live.

Breaking changes: none. The only behaviour change: with a dashboard open, serve may now use its full 5-second drain on stop.
