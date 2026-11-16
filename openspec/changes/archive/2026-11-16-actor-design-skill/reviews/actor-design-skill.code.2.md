---
id: actor-design-skill.code.2
kind: code
change: actor-design-skill
round: 2
base: ef91a7b4d9fd5fb82c80db7649e1085f892e30d4
head: 53f27d21a12d047d1abbe056b4e296949c134a01
previous: d1d9e847493b3dd7e21f9b89a957e1eb9a445ab9
requested: 2026-11-16T15:36:31+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-design-skill.code.2 <file>`.

## Note from the requester

Version: 0.19.3

Round 2. Fixes for the round-1 blocking findings; head 53f27d21a12d (previous head d1d9e84), one commit. Not rebased onto main yet.

Finding 1 (resume after a driver update):
- New `test_a_format_two_resume_after_a_driver_update_runs_the_pinned_driver` in scripts/test_gate_release.py; the checkout-drift test stays. It saves format-2 digests; commits a changed scripts/gate.py and scripts/package-skill.py in the caller's checkout; checks the updated driver is refused ("run the gate of … itself") with no state change, no GitHub writes, no release and the remote tag unchanged; restores the pinned gate.py and resumes: published, both SOURCE.json files and member names verified, digests equal the saved ones; the caller's later packager is never used.
- Spec scenario "Resume format 2 after a driver update" reworded: an updated driver is refused and resuming runs the pinned driver; still verifies against the pinned commit and saved digests and fails on a mismatch instead of rewriting.

Finding 2 (delegation in Revise and Finish):
- skills/actor-design/SKILL.md: within the delegated scope a revised result and the no-actors ending are `delegated`, recorded under Decisions with reason and uncertainty; outside the scope the default (draft, show, wait) stays.
- Two staged cases, `delegated-no-actors` and `delegated-revision` (skill.rs now requires 16). A fresh subagent ran both (behavioral-review.md, round 3): full delegation ended at step 1 with no actors without waiting; an in-scope change revised steps 5 and 6 and kept them delegated. A first variant changed the user's own steps 1-4 and correctly fell back to the default; the case was changed to an in-scope change.

Open judgement calls from the subagent, left as they are: when a step counts as directly changed rather than only affected; whether delegated later steps may be revised before the user reconfirms an earlier draft; where a fact about an outside system belongs before step 8.

Checks on 53f27d2: skill test 16, docs test 11, script tests 89, fmt clean, openspec validate --strict valid; gate.py run --e2e with Node 26: full pass and e2e pass (docs record missing, expected).

Breaking: none. Docs: handed off, no blockers (handoff updated for delegation, 16 cases and the driver-update test).
