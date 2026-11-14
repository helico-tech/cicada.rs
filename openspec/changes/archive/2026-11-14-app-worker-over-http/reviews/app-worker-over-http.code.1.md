---
id: app-worker-over-http.code.1
kind: code
change: app-worker-over-http
round: 1
base: b9773a3f6f898d06fc4158093ceab986069e8d5f
head: 53bc7ab379c0e89156ed56b8d0064bf9804ed5da
requested: 2026-11-14T07:49:20+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done app-worker-over-http.code.1 <file>`.

## Note from the requester

Version: 0.16.17

Builder's report:

Tasks (all done):
1. Worker mode resolution (store or HTTP) first, maps only the settings a remote worker can use, refuses server-owned flags and on_start, redacts credentials in Debug.
2. Shared hosting for types and observers, a client with no redirects, the readiness wait, start/stop handling.
3. crates/cicada-app/tests/worker_http.rs, 11 tests (5 consecutive passing runs).
4. Docs (architecture, app guide, operations, HTTP guide, crate docs, README), CLI reference, OpenAPI version, CHANGELOG, version 0.16.17. ADR 0042 and its index row already existed.
5. Finishing checks.

Checks: cargo fmt --all --check passes; cargo clippy --workspace --all-targets -- -D warnings clean; cargo test --workspace with PostgreSQL: 2707 passed, 0 failed (incl. 12 new virtual-time tests in src/node.rs and 9 mode/settings tests in src/settings.rs); cargo check -p cicada-app with no features and with sqlite, postgres, tui each alone: clean; openspec validate app-worker-over-http --strict: valid. Not run: Docker E2E.

Departures: the integration tests use real sockets in real time with wait-until checks and no timing claims; all timing and race checks are in the virtual-time unit tests (design: "socket framing needs no timing claims").
Finding outside scope: CICADA_NODE_ID containing NUL is not caught as a usage error; it parses and the Worker refuses it at start (exit 1).

Example:
  myapp worker --server https://actors.internal --token "$TOKEN"   # or CICADA_SERVER and CICADA_TOKEN
  myapp worker --context prod --types sync
  myapp --server http://x worker --store a.db    # exit 2: not both

New concepts: worker mode (--store vs --server/--context; with no flag a non-blank CICADA_STORE wins); server-owned settings refused over HTTP, their env vars ignored; readiness wait (lists server types once a second while unavailable, no new try after 60 s, ~90 s max excluding registration).
Breaking: none; only messages change (worker --token on the store and worker -o give new usage errors; a worker with no mode gets a new hint).
