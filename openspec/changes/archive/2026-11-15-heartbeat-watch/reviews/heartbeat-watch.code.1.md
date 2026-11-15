---
id: heartbeat-watch.code.1
kind: code
change: heartbeat-watch
round: 1
base: baa4568b9a472b36e17082ceb4d4296d748b2057
head: 2351f94462d8b8896cea2265a4509b07c38d9ecf
requested: 2026-11-15T10:32:05+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done heartbeat-watch.code.1 <file>`.

## Note from the requester

Version: 0.18.10

Builder's report:

Tasks 1–6 done: types and validated state; Beat, Rearm and Expire with tests; failure, metadata, depth and contract proofs (catalog from eight to nine contracts; the new docs/reference/contracts/cicada.heartbeat-watch.json is the only contract change); example 19 in examples/src/heartbeat.rs with its skill route; docs handoff, changelog, version, skill version and openapi.json; finishing checks.

Checks (PostgreSQL on 55432): fmt --check and clippy -D warnings clean; openspec validate --strict valid; cargo test --workspace 2944 passed, 0 failed; `gate.py checks --scope full` exit 0, 3093 passed, 0 failed; `gate.py checks --scope e2e` exit 0. After rebasing onto a docs commit on main: doctests 29, docs test 11, skill test 8 passed. No gate evidence recorded (the build loop asked the builder not to run `gate.py run`).

Gaps the builder names: the self-target refusal on a valid Expire is never run by a test; the public API cannot produce it (Beat and Rearm refuse a watch whose alarm targets itself, so no valid timer can exist for it); those refusals are tested. Module named heartbeat_watch after the type name. Test `the_contracts_of_the_eight_types…` renamed to `…nine…` (no doc names it). Open question (not a blocker; the build loop will take it to the owner): the sample service hosts every other ready-made actor but not this one; the tasks did not ask for it.

New concepts: heartbeat watch (starts waiting; each Beat restarts the silence count from when the watch handles it; when the silence runs out it sends the alarm once and becomes silent; the next Beat starts a new round and tells nobody); Rearm (re-arms a purged, cancelled or failed timer at the same deadline without faking a Beat; a retry or redrive of the old timer gets a new id and is ignored).
Breaking: none. Docs: handed off, no blockers (actors guide section, recovery notes, architecture row, install pin, and the `#heartbeat-watch` skill route once that heading exists).
