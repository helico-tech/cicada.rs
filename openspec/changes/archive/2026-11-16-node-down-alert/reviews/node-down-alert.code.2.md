---
id: node-down-alert.code.2
kind: code
change: node-down-alert
round: 2
base: 1b23e4f85aadf712a162aba378c90ef4ddd9e341
head: f0213c0c0bad1c6a4e5c48bacbc7b3e1c29db44d
previous: 2ea550fb05c98171e13f9a7d0aeaadbc732ecc81
requested: 2026-11-16T22:35:55+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done node-down-alert.code.2 <file>`.

## Note from the requester

Version: 0.20.2

Round 2. Fix for the round-1 blocking finding; head f0213c0c0bad (previous head 2ea550f). Not rebased onto main yet.

Fix (the build loop's pick: deterministic on paused time):
- `an_unstarted_system_reads_the_real_node_rows_without_a_row_of_its_own` now runs with start_paused = true. A small `crash` helper starts web-1 in a Tokio runtime of its own on a separate thread, waits until its row exists, then drops that runtime: the heartbeat task dies and the row stays, with no production fault hook (the memory store uses std locks and starts no tasks).
- The test moves the store clock 120 s and checks the alert (node, lastSeen, detectedAt, label, version); restarts web-1 with the same id (seen alive, forgotten silently); then a clean stop (absence, no second message). Store time moves only when the test moves it, so a heartbeat writes the time the test set whenever it fires. The reader still has no row.
- docs-handoff.md test description corrected: 12 cases on paused time, 11 with a scripted source and 1 against real rows.
- The stronger replay test (a commit lost after effects were computed) was not added: not cheap with the public API.

Checks on f0213c0: cicada-actors --test node_down 12 passed on each of 8 runs; the real-row test alone 40/40; clippy -D warnings clean; --doc 34; --test docs 11; --test skill 18; docs/guide/actors.md equals 7052813 (checked again by the build loop). gate.py run with Node 26: full pass, 3267 tests; --e2e pass. gate.py status: full and e2e pass of f0213c0c0bad.

Docs: owner fixes integrated (unchanged since round 1). Breaking: none.
