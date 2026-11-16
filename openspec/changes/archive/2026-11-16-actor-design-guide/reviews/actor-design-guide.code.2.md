---
id: actor-design-guide.code.2
kind: code
change: actor-design-guide
round: 2
base: 6e589e6c1c33d3269376d6c082603a282e799f40
head: a5c4fc3c926a62905901c84b092c4d575d148750
previous: 6e69ae7f05390b050f466674410b8982fe529393
requested: 2026-11-16T14:16:05+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-design-guide.code.2 <file>`.

## Note from the requester

Version: 0.19.2

Round 2. Fixes for the round-1 blocking findings; head a5c4fc3c926a (previous head 6e69ae7). Not rebased onto main yet.

Finding 1 (demo race), commit b8bf051:
- The wait moved into `websocket_session::emit_and_wait`: it sends the Emit with a known message id (a refused send is an error), waits for that Emit's recorded outcome (anything other than done is an error), then reads producer state until nothing is pending. The whole wait has a limit; the demo uses a 200 ms poll and 30 s.
- Tests (paused time): `the_demo_emit_waits_for_the_answer_to_its_own_emit_when_turns_run_late` (turns held back past several polls while an earlier answer sits in state; also a fast answer; the old loop fails it) and `the_demo_emit_reports_a_refused_or_failed_emit_and_the_limit`.

Finding 2 (browser client untested), commit a5c4fc3. Owner decision 2026-11-16: option A, Node's built-in test runner, Node 26 minimum.
- `examples/tests/websocket_session_client.test.mjs`: node:test, node:assert, node:vm only, no packages. Runs the HTML's <script> text unchanged with fakes for WebSocket, localStorage, timers, location and the page.
- `scripts/ci/client-tests.sh` runs it; `scripts/gate-policy.json` adds it to the full checks as `client-tests`. Fails clearly when node is missing or older than 26; never skips. `scripts/test_gate_policy.py` covers both failures.
- 9 cases pass on Node 26.13.1: route; contiguous apply with a per-stream cursor that survives reload; duplicate re-acked, not re-applied; gap closes and reconnects once after 1 s without skipping; old-socket messages and close ignored; Hello ahead of a lost cursor shows recovery, closes, no ack, no reconnect; Hello with cursor >= acked re-acks, a fresh client sends nothing; other stream ignored; lost connection reconnects after 1 s. Removing the old-socket guard makes exactly that case fail. HTML unchanged; no client bug found.
- The gate passes the caller's PATH through; the owner set nvm's default to Node 26 on this machine (2026-11-16).

Checks on a5c4fc3: --doc 30, --test docs 11, --test skill 10, --test websocket_session 28 (20 repeat runs green), script tests 83 OK, clippy -D warnings, fmt, openspec validate --strict. gate.py run --e2e with Node 26: full pass (log shows client-tests, 9 passed) and e2e pass.

Docs: owner fixes integrated. The six docs paths still equal 147f05a byte for byte (checked by the build loop). Handoff gains non-blocking item 4: Node 26 as a gate prerequisite in docs/how-to/releasing.md and AGENTS.md (docs session, after landing). The handoff caveat now says what the Node test covers and that no real browser ran.

Breaking: none for library users. The gate now needs Node 26 or later.
