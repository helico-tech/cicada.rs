---
id: actor-design-review.code.2
kind: code
change: actor-design-review
round: 2
base: 3c88916c954924de9693b23806ccf6a0b6b03b2b
head: 0c77c483234417332aaba9443e8300352e0c7644
previous: a993a8ad12c751c6ae220ce9fb513869df0a430b
requested: 2026-11-16T16:55:03+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-design-review.code.2 <file>`.

## Note from the requester

Version: 0.19.4

Round 2. Fix for the round-1 blocking finding; head 0c77c4832344 (previous head a993a8a). Not rebased onto main yet.

Fix (the build loop's pick: R-001 stays open):
- skills/actor-design/SKILL.md, Dispositions (section 8): one new sentence. A change whose effect depends on an outside system's unverified behaviour does not resolve a finding; the change is recorded under History, the finding stays open, and the evidence is asked for under Open questions.
- examples/tests/skill/actor-design-evals.json, case review-changed-source-resume: expect R-001 open, with the key change cited under History and the earlier trace kept, and an open question whether and how long the provider deduplicates by that key; avoid "Marking R-001 resolved on a code comment about the provider". R-001's id and trace and the user's note on R-002 unchanged.
- Reruns with fresh agents against the final skill (all wording fixes included), recorded in behavioral-review.md "Round 2": resume (R-001 open and high with a question on the provider's deduplication; R-002 keeps the note; R-003 withdrawn; receipt email R-004); sound case with the corrected fixture (no findings, cart-plus-checkout tradeoff accepted, four open questions). The judgement call about resolving on a comment is now settled; docs-handoff.md says the same.

Still disclosed: four cases not run by an agent; Git blocked in the agents' folders, revisions faked with a file.

Checks on 0c77c48: --test skill 18, --test docs 11, openspec validate --strict valid; gate.py status: full pass and e2e pass of 0c77c4832344.

Breaking: none. Docs: handed off, no blockers.
