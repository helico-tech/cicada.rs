---
id: actor-design-skill.code.1
kind: code
change: actor-design-skill
round: 1
base: ef91a7b4d9fd5fb82c80db7649e1085f892e30d4
head: d1d9e847493b3dd7e21f9b89a957e1eb9a445ab9
requested: 2026-11-16T15:10:22+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done actor-design-skill.code.1 <file>`.

## Note from the requester

Version: 0.19.3

Builder report (feat/actor-design-skill, head d1d9e847493b; base ef91a7b, main has since gained docs-only commits up to 230eac7; not rebased yet):

Tasks: all six done. 1 the skill and its template; 2 structural checks and 14 staged test cases; 3 the two-skill archive; 4 the release check and resuming a release; 5 handoff and version 0.19.3; 6 finishing.

Checks: gate.py status full pass and e2e pass for d1d9e847493b (docs record missing, expected for a builder). cargo test --workspace with PostgreSQL 3016 passed; fmt and clippy clean; skill test 16 passed; script tests 88 passed; openspec validate --strict valid; claude plugin validate passed.
Not checked: Claude Code actually finding and loading /cicada:actor-design (plugin not installed into a session). Behavioural tryouts: two fresh subagents played the skill on 7 of the 14 cases; findings and gaps in openspec/changes/actor-design-skill/behavioral-review.md.

Departures and open questions (for review):
- Seven small clarifications to the skill after the first tryout (how a design with no actors ends, how far a delegation reaches, when the document is created, and similar).
- Three judgement calls left for review, e.g. whether a revised step that was never confirmed becomes `draft` or `needs review`.
- "Resume after a driver update" cannot happen as written: the gate refuses a gate.py that differs from the pinned commit's. Tested instead as: the caller's checkout moves on, the saved archive checksums are re-checked against the pinned commit, and a changed checksum fails. Please judge whether this meets the scenario.
- .claude/skills/build-loop/builder.md still names only one skill's version (outside the change; the build loop takes it to the owner).

New concepts: the actor-design skill (works out a design with the user one step at a time, saved to docs/design/actors.md, waits for confirmation per step unless the user lets it decide); archive format 2 (a tag's scripts/skill-archive-format selects a two-skill archive; older tags package as before).

Breaking: none for library users. Docs: handed off, no blockers (docs-handoff.md).
