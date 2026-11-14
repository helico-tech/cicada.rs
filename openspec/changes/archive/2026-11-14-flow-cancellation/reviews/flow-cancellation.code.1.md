---
id: flow-cancellation.code.1
kind: code
change: flow-cancellation
round: 1
base: 2f31b9398a60b3f33b966362a22f007e1e04004f
head: c8eb7fb0defe8aad34c5956c1c0f809c30fd22ae
requested: 2026-11-14T18:26:26+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-cancellation.code.1 <file>`.

## Note from the requester

Version: 0.18.4

Owner rule for this build (decision 24a): README and hand-written docs belong to the docs session; user-facing prose is in openspec/changes/flow-cancellation/docs-handoff.md. Review the handoff instead of guide edits; the guide's compiled example lives as a doctest on `FlowMsg`.

Builder's report:

Tasks 1–5 done. A Cancel to a running run cancels the armed Continue (unless that message is the Cancel itself), sets status cancelled and clears `armed` in one commit. The status is checked before the message type, so a cancelling run refuses all three messages for good. Tests cover waits, sleeps, joins, races with replies, deadlines, queued continues, running handlers, retries and pauses; rejected, lost and unanswered commits are injected through a TurnSource wrapper in a Worker. Boundary tests: observers, change hints, watches, other senders, delete, create-and-cancel, operator cancels, refusal by an older host (simulated by a test actor of the same contract; no real 0.18.3 build ran).

Checks: fmt --check and clippy -D warnings clean; cargo test --workspace with PostgreSQL 2865 passed, 0 failed; openspec validate --strict valid; docs, doctest and skill tests pass; every contract hash unchanged, the test flow's snapshot without descriptions equals main's. Not run: Docker E2E and per-feature checks (release checks; only cicada-actors changed).

Departure: the first commit leaves two snapshot-text tests failing until the third commit regenerates them; every commit builds.

New concepts: cancelling a flow run (FlowMsg::Cancel ends a running run in its own turn, keeps all saved progress, never interrupts a running turn, undoes no effects, runs no cleanup); the cancelled status (final; ignores Continue, Reply and Cancel; readable until deleted; a new job needs a new run key).
Breaking: none to API or wire. A 0.18.3 host refuses Cancel for good despite equal hashes: upgrade every host and worker of a flow type before anyone sends it.
