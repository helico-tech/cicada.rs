---
id: flow-finalizer.code.1
kind: code
change: flow-finalizer
round: 1
base: 58c01805ba276dd6ef5918016b5b530cf3e9738d
head: 758ccaf2c01984e0534e7042f980717a3b9c3d24
requested: 2026-11-14T19:22:36+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-finalizer.code.1 <file>`.

## Note from the requester

Version: 0.18.5

Owner rule for this build (decision 24a): README and hand-written docs belong to the docs session; user-facing prose is in openspec/changes/flow-finalizer/docs-handoff.md (to be applied after flow-cancellation's handoff). Review the handoff instead of guide edits.

Builder's report:

Tasks 1–6 done. Cancel marks a running run cancelling and arms a cleanup Continue without running flow code; the phase comes from the saved status; cleanup keys are <run>/cancel/<id>; only a successful cleanup marks cancelled. Lookups (main_key, main_result, main_request, main_reply, main_journal) and MainReply; cleanup waits over main requests no closed main wait names, and reply routing while cancelling; recovery and observation tests; example 18 (examples/src/flow_resources.rs, tests in examples/tests/flow_resources.rs). Rejected, lost and unanswered commits are injected through a TurnSource wrapper.

Checks: fmt --check and clippy -D warnings clean; cargo test --workspace with PostgreSQL 2900 passed, 0 failed. Two tests were added after that run (an older host and the default hook skip the cleanup; a cleanup reply refused by an older host) and only cicada-actors was re-run for them: 479 passed. openspec validate --strict valid; contract hash and description-stripped schema unchanged against main. No change under crates/cicada, cicada-axum, cicada-client or cicada-app. Not run: per-feature checks and Docker E2E (release checks).

Departures: eleven existing cancellation tests now expect two turns (cancel, then cleanup), as the new spec says. Example 18 needed one new path in the flows row of skills/cicada/SKILL.md (the skill test requires every example routed).

Version: the build loop chose a patch (0.18.5). The API change is additive (Flow::on_cancel has a default), wire and hashes are unchanged, and 0.18.4's spec already announced that the finalizer piece defers cancelled until cleanup finishes. Behavior visible to a caller: Cancel now takes at least two turns; a status read right after the Cancel turn shows cancelling. Flag it if you think this needs a minor bump.

New concepts: Flow::on_cancel / Cleanup (cleanup after Cancel, default no-op; Step's helpers with its own journal and ids; a failure keeps the run cancelling, public Continue recovers, a second Cancel does nothing); main lookups (read what main saved, fixed once cancelling; a missing result does not prove no effect, so release by key); MainReply (Open, Replied(T), Closed, decided only by closed main waits).
Upgrade: every host and worker of a flow type must run 0.18.5 and the same on_cancel code; a 0.18.4 node or default-hook code marks cancelled with no cleanup, and a 0.18.4 node refuses every message to a cancelling run for good.
