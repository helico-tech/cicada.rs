---
id: actor-design-review.code.1
kind: code
change: actor-design-review
round: 1
base: 3c88916c954924de9693b23806ccf6a0b6b03b2b
head: a993a8ad12c751c6ae220ce9fb513869df0a430b
requested: 2026-11-16T16:34:08+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-design-review.code.1 <file>`.

## Note from the requester

Version: 0.19.4

Builder report (feat/actor-design-review, head a993a8a; base 3c88916; main has since gained the docs commit 115a636; not rebased yet):

Tasks: all five done.
1. Review mode in skills/actor-design/SKILL.md: mode choice, four review phases, claim tags, diagrams before any finding, seven areas, finding format; the cicada skill names both modes.
2. Saved reports and resume: report outline in section 8, default docs/design/actors-review.md; resume marks findings stale when code changed, ids never reused, a finding is resolved only with new evidence.
3. Test cases: 15 review cases, test-only input files under examples/tests/skill/review-fixtures/, new checks in skill.rs.
4. Handoff and release. 5. Finishing.

Checks: --test skill 18 passed; --test docs 11; --doc 30; cargo test --workspace without PostgreSQL 3018 passed; openspec validate --strict valid. gate.py status: "checks: yes, docs pass over the full pass of d0d770087db8"; "e2e: yes, e2e pass of the full-checked base d0d770087db8" (a993a8a only ticks tasks.md).

Behavioural review (behavioral-review.md): three fresh agents ran 11 of 15 cases. Not run: combined-request, old-source, mixed-source, no-repository. The runs used the skill before the wording fixes made from their feedback, so those fixes are untested. Git was blocked in the agents' test folders, so the resume case faked its two revisions with a file.

Beyond the design (builder's account, please judge):
- Eight small wording fixes after the runs, said to be inside the design (e.g. a finished review goes back to Check when code changes; a code comment is documented intent, not proof of what an outside system does).
- The Recovery area also points to docs/architecture.md#turn-semantics.
- The "sound" fixture had a real bug (checkout never cleared its stock reservations), found by an agent; the fixture was fixed and not re-run.

New concepts: review mode (reads only, draws the system first, then judges); finding ids R-001 and on, stable across reruns, resolved only with new evidence.

Breaking: none. Docs: handed off, no blockers; the handoff builds on the 0.19.3 handoff (whose prose has since been published by the docs session at 115a636).
