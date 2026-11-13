---
id: flow-sleep.code.1
kind: code
change: flow-sleep
round: 1
base: bcddf0fe81f8013282c11b9f653f169225fb8226
head: 1642ad41113569b79bafeb15a521591b0df570ac
requested: 2026-11-13T14:48:48+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done flow-sleep.code.1 <file>`.

## Note from the requester

Version: 0.16.6

Builder report (round 1)

flow-sleep is built and committed on branch feat/flow-sleep, 4 commits on top of bcddf0f. Nothing pushed, merged or tagged.

Tasks
- Task 1, done: Step::sleep and the scheduling of a new or not-yet-due sleep, with 6 library tests and an API doctest.
- Task 2, done: 13 new virtual-time tests in crates/cicada-actors/tests/flow.rs. The test sign-up flow in wire_formats.rs now sleeps; its contract fixture is unchanged.
  Addition asked by the build loop (note 2 of the durable-flows review): the lost-progress test is now a_step_effect_lost_to_a_turn_timeout_is_called_again_with_the_same_key; its doc and message say the call is made and the timeout drops it before it responds. Assertions unchanged.
- Task 3, done: the example, the actors guide (new section "Durable sleep"), the changelog and version 0.16.6.
- Task 4, done: finishing checks.

Checks
- cargo fmt --all --check: clean.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2256 passed, 0 failed.
- openspec validate flow-sleep --strict: valid.
- No contract fixture, snapshot or contract text changed. One new state sample: sample-service/tests/states/flow-onboarding/sleeping.json.
- Not run: the Docker E2E suite.

Departures and open questions
- The doc comment on JournalEntry::Sleep still says the sleep is "reserved for … a later release". Left, because doc comments are part of the pinned contract text and the design forbids changing it; fixing it means regenerating the fixtures. (The build loop takes this to the owner as a later change.)
- Tokio's test timer rounds to whole milliseconds, so "nothing runs before the wake time" is checked at the wake time minus 1 ms; the microsecond rounding is checked in the library tests.

New concepts
- Durable sleep: the first call saves a fixed wake time in the run's journal and ends the turn; the next Continue is due at that time, so no handler waits.
- Saved wake time decides: on replay the sleep compares store time with the saved time and ignores the duration in the code. An early Continue, deliver-now, retry or redrive sets the timer again for the same time.

Breaking changes: none. Deploy 0.16.6, with the flow code that sleeps, to every node that hosts the flow before starting runs that sleep.
