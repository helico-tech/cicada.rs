---
id: sample-heartbeat-watch.code.2
kind: code
change: sample-heartbeat-watch
round: 2
base: a1cff63063faa6244d605ef648dc3487e71810a2
head: e0f360b9094a2c1e48b39b3679c78265d1a72d1a
previous: d87ee2b1fd0e5d113dc3cd32fa2e9c2279e61afd
requested: 2026-11-17T11:23:38+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done sample-heartbeat-watch.code.2 <file>`.

## Note from the requester

Version: 0.20.4. Round 2, after code.1 (fix needed: stale handoff Status).

Fix: e0f360b, Markdown only, changes only openspec/changes/sample-heartbeat-watch/docs-handoff.md. The Status section now records the owner commit 28519faffa82ef22d8f07fb022c3074df9c04b1e, its only path docs/guide/app.md, the whole-file checkout in a8beaa8 (which ticked task 3), and the verification: the docs session's checks of that exact file on d87ee2b, gate full plus E2E on d87ee2b, docs run on the Markdown commits on top, and openspec validate --strict.

Gate: gate.py status for e0f360b shows "checks: yes, docs pass over the full pass of d87ee2b1fd0e" and "e2e: yes, e2e pass of the full-checked base d87ee2b1fd0e".

No other changes since 48df3a0.
