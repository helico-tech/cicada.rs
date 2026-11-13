---
id: otel-timer-traces.code.2
kind: code
change: otel-timer-traces
round: 2
base: 9662c2c43c5e885eb69e0548d76ff4f7b11f2fe9
head: 2f9ddd78cdd90130c2cc9480ca5a64c4b097c191
previous: 4213c2382d68310f1229736bd0b06b18a1e2c6f9
requested: 2026-11-13T15:37:42+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done otel-timer-traces.code.2 <file>`.

## Note from the requester

Version: 0.16.8

Builder report (round 2: fix of round 1)

Blocking finding 1 of round 1 is fixed in 2f9ddd7 on feat/otel-timer-traces.

The fix: a new test an_old_node_ignores_the_link_headers_of_a_new_timer_and_starts_a_root_without_a_link in crates/cicada-otel/tests/actor_telemetry.rs.
- A new node first sends a timer; the test checks its only header is cicada-link-traceparent.
- A frozen copy of the observer as it was before 0.16.8 (LegacyTelemetry) gets those exact headers on a second node. It takes the parent from the plain headers only, makes the same kind of span and knows nothing of links.
- It asserts the message ends done and the handler saw the new headers; the old observer exports one span, with no parent; that span has no links.

Checks
- cargo test -p cicada-otel: 32 passed, 0 failed (1 more than before), plus 1 doctest.
- cargo fmt --all --check: passed.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2328 passed, 0 failed.

The optional lost-claim note: not done. A traced first attempt followed by a takeover needs the first turn's lease to run out while its handler runs; the runtime keeps renewing it, so that needs the private fault-injection hooks in crates/cicada/src/tests. claim_and_abandon stays; the re-run (attempt 2) is still checked to be a root linked to the sender.

Breaking changes: none beyond round 1 (timer turns start their own traces, as intended).
