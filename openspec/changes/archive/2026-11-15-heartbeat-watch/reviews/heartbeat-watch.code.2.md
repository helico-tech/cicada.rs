---
id: heartbeat-watch.code.2
kind: code
change: heartbeat-watch
round: 2
base: baa4568b9a472b36e17082ceb4d4296d748b2057
head: c013ab2fae59d05a76740ef6ec58cf89eaec0148
previous: 2351f94462d8b8896cea2265a4509b07c38d9ecf
requested: 2026-11-15T10:40:11+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done heartbeat-watch.code.2 <file>`.

## Note from the requester

Version: 0.18.10

Round 2: fixes only the two round 1 blockers (commit c013ab2, range 2351f94..c013ab2; tests and docs handoff only). Round 1's report is .reviews/heartbeat-watch.code.1.result.md.

Builder's report:

1. Schema and decoder boundaries: new test `a_shape_valid_but_inconsistent_state_accepted_by_a_node_without_the_watch_fails_its_turn_with_cause_state` in crates/cicada-actors/tests/heartbeat_watch.rs. It creates a watch through the HTTP API of a node on the same store that does not host the watch (schema the only check): an alive state without armed is accepted (201); a wrong shape (status "asleep") is refused (422) and nothing is created; the decoder refuses the alive state; on the hosting node Beat, Rearm and Expire each fail with FailureCause::State; state JSON and version unchanged, no timer pending, no alarm at the target.
2. Self target, eligible timer: new test `an_eligible_timer_of_a_watch_whose_target_is_itself_is_refused_with_no_effects`. It leases the watch's turn through the public lease API and completes it with an alive self-target state and a send of an Expire whose id equals armed; the pending timer has that id and the watch as sender; the real actor refuses it for good (Refused, "to itself"); state, version and mailboxes unchanged, no new timer, no send. Mutation check: with the refusal line removed from `expire` the test failed; restored. The forged/inactive no-op test is unchanged.
3. Handoff: test count and a round-1 note; no user-visible text changed.

Checks: fmt --check and clippy -D warnings clean; cargo test -p cicada-actors with PostgreSQL 516 passed, 0 failed; --test docs 11 passed. Full workspace not re-run (only tests changed); no gate evidence for this head (the release runs the gate).
