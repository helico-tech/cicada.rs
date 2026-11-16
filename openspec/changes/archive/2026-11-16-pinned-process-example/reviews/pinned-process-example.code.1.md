---
id: pinned-process-example.code.1
kind: code
change: pinned-process-example
round: 1
base: 52ac854c793f7d5572be23c74237f977f64d7d84
head: fcba7e4499bf4a08fea1b062c50346e5b7baa957
requested: 2026-11-16T20:54:20+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done pinned-process-example.code.1 <file>`.

## Note from the requester

Version: 0.20.1

Builder report (feat/pinned-process-example, head fcba7e4; base 52ac854; main has since gained the docs commit 278cfe5; not rebased yet):

Tasks: all six done.
1. Run actor and host protocol (examples/src/pinned_process.rs).
2. Observation pump, startup reconcile and recovery tests (examples/tests/pinned_process.rs).
3. Demo host examples/examples/pinned_process.rs and real-process test examples/tests/pinned_process_os.rs.
4. Module exported, both skills route to the example, docs-handoff.md written.
5. Version 0.20.1, changelog, OpenAPI regenerated; ADR 0053 already matched, unchanged.
6. openspec validate --strict and the gate.

Checks: fmt, clippy -D warnings clean; cargo test --workspace with PostgreSQL 3079 passed; pinned_process 18/18 (a deliberate break of slot reuse and of the final-status check made 5 fail); pinned_process_os 3/3 on macOS only (Linux untested here); doctests 30, docs 11, skill 18. Gate: full and E2E passed on df9a5c5; after the task-tick commit, "checks: yes, docs pass over the full pass of df9a5c5caa23", "e2e: yes".
Demo run by hand: normal exit, Ctrl-C cleanup, kill -9 then restart shows NeedsReconciliation.
Not shown: cleanup after an abrupt host death (after kill -9 the demo child exited only because its input closed; not promised).

Departures (please judge):
- The OS adapter (OsBackend, child_main) lives in the library module, not the binary, so the OS test uses the same adapter.
- Tokio's process feature was already on workspace-wide; nothing changed there.
- The OS test gives its shutdown a 2 s budget instead of 5 s, because no pump runs there and shutdown would otherwise wait out its whole budget.

New concepts: host epoch (a fresh id per host process start, separate from the stable node id; a turn from another epoch or node acts on no process); slot (the host's record of one run, at most 32, never reused, spawns at most once, so a replay after a lost commit finds it instead of spawning again); NeedsReconciliation (process state unknown; terminal; check outside Cicada, then delete and use a new key).

Breaking: none. Docs: handed off, no blockers.
