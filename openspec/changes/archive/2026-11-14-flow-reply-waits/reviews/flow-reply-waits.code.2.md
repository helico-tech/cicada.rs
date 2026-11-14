---
id: flow-reply-waits.code.2
kind: code
change: flow-reply-waits
round: 2
base: 8605589a613e436c2fd318325cf784d83a834fbc
head: f9d0bc40e4a281ab08b1c524792f82b06514a6a0
previous: 3fd6aea05819569f2e9517c82362a447954b973b
requested: 2026-11-14T14:42:15+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-reply-waits.code.2 <file>`.

## Note from the requester

Version: 0.18.1

Round 2: adds the two missing scenario tests, in commit f9d0bc4. Only tests changed; the implementation and the wire doc comments are as in round 1 (the owner chose to keep the corrected descriptions).

1. Reserved active wait: `a_reply_that_would_close_a_reserved_wait_is_refused_for_good_and_saves_nothing` (crates/cicada-actors/tests/flow.rs). Each run starts from supplied progress whose open wait is a join, a first wait or a deadline wait; the saved target responder/r1 sends a valid reply for request m1. Asserts per case: the Reply fails for good on its first attempt, from r1, with the later-feature message, the watcher told with cause `refused`; state JSON and version unchanged; no reply saved, the wait has no outcome; `armed` stays null and nothing pending for the run; no flow code ran.
2. Sender boundary:
   - Remote worker: `a_reply_from_a_target_leased_by_a_remote_worker_passes_the_sender_check`. The flow runs on a test node; the responder runs in a separate worker that leases its turns from the node's ActorSystem through the core TurnSource. The worker's completed turn produces a Reply with sender responder/r1; the run accepts the value, closes the wait, runs the next step and ends done.
   - Dead-letter redrive: `a_reply_redriven_by_the_dead_letters_is_ignored`. The target's reply is 62 levels deep so it fails for good and dead letters keep it; on redrive the new Reply comes from cicada.dead-letters/_ and ends done (ignored, so the sender check runs before the depth check); nothing written, the run stays parked.

Checks: fmt --check and clippy -D warnings pass; cargo test --workspace --no-fail-fast with PostgreSQL 2789 passed, 0 failed.

Please check both round 1 findings are closed and the tests add no new problem.
