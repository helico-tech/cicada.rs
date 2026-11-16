---
id: node-down-alert.code.1
kind: code
change: node-down-alert
round: 1
base: 1b23e4f85aadf712a162aba378c90ef4ddd9e341
head: 2ea550fb05c98171e13f9a7d0aeaadbc732ecc81
requested: 2026-11-16T22:22:00+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done node-down-alert.code.1 <file>`.

## Note from the requester

Version: 0.20.2

Builder report (feat/node-down-alert, head 2ea550fb05c9; base 1b23e4f; main has since gained the docs commit 9edd1ff; not rebased yet):

Tasks: all six done.
1. Monitor contract and source (crates/cicada-actors/src/node_down.rs; NodeSnapshotSource, implemented for ActorSystem).
2. Polling and deduplication: fixed 10 s durable poll, 5 s reads, 1000-row cap; one alert per (startedAt, lastSeen) watermark; dead at as_of - lastSeen >= 120 s; no recovery event.
3. Contract (docs/reference/contracts/cicada.node-down.json, wire_formats.rs) and example 22 (examples/src/node_down.rs): a pager told of dead nodes through a topic, of monitor failures through a watch, with lastProblem read separately.
4. Routes in both consumer skills, docs handoff.
5. ADR 0054 (matched, unchanged), version 0.20.2, changelog.
6. Finishing.

Checks on 2ea550f: --doc 34, --test docs 11, --test skill 18, --test node_down 4; gate.py run with Node 26: full pass, 3267 tests; gate.py run --e2e: e2e pass. gate.py status: full and e2e pass of 2ea550fb05c9.

Departure (please judge): the test that reads real node rows runs in real time with a hand-moved store clock (VirtualClock), not paused tokio time, because with paused time the started node's heartbeat refreshes its row and hides the death.

New concepts: NodeDown (published through a topic; node id, startedAt, lastSeen, detectedAt = sample time, optional hostname/version/label); watermark; lastProblem (caught read failures unavailable/timeout/invalid/tooManyNodes, which never fail a turn); NodeSnapshotSource.

Docs: owner fixes integrated (ADR 0045). Docs commit 7052813523e69d9c294c7b1ca3005f7c3600c4bb, one path, docs/guide/actors.md (roster row and ## Node-down alerts), taken whole with git checkout, no merge, folded into afed58e (the commit that routes both skills to the anchor). The build loop checked: the docs commit holds only that path; it is byte-equal on the branch to 7052813 and the docs branch; main did not touch it since the base. Please check the section still describes the code. Handoff items 2-5 (operations, architecture, core guide) remain proposed prose for after landing; none blocks a check.

Breaking: none (additive).
