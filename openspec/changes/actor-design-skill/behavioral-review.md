# Behavioral review of actor-design (builder's evidence)

What was exercised while building, and what remains unverified. This is evidence for the reviewer, not a claim of
model compliance; CI checks only the structure of the skill and its staged cases.

## How

A fresh subagent (Claude Opus 5.5, the builder's model family) read `skills/actor-design/SKILL.md` and its template
and played the agent for staged cases, one user turn at a time without looking ahead, in temporary git repositories.
The resolver was not run (the temporary projects have no Cicada dependency): every case assumed a report of status
`ok`, version 0.19.2, `skill_source: null` (plugin layout), with the root set to this worktree, so the agent read the
real design guide. The builder judged the transcripts against `examples/tests/skill/actor-design-evals.json`.

## Round 1 (skill at commit 00fe539)

| Case | Result |
| --- | --- |
| fresh-design | Created the document from the template on starting; three questions about step 1 only; step 1 draft with labelled hypotheses and a context drawing without actors; confirmed only on the user's yes, then step 2 draft with a timeline. Used the user's words (order, stock, card, ship). |
| not-an-actor | Asked whether actors fit; drafted "no actors" with reasons and a plain flowchart; asked the user to confirm instead of continuing. |
| confirmation-and-delegation | A question about refunds was recorded as an open question; step 3 stayed draft. Delegation of cards and protocols was recorded under Decisions; the agent asked whether step 4 was in scope instead of assuming it. |
| resume-changed-requirement | Read the document, rechecked the source, continued at step 6 with its open questions; a new requirement sent it back to step 1 (draft) and marked steps 2 to 6 needs review, keeping their text. |
| missing-sibling | Reported the missing sibling, read `Cargo.lock` itself, refused to use a newer guide, started steps 1 and 2 and said steps from 3 on wait for a readable source. |

No premature advancement, invented actors or claimed confirmations were seen. The agent named seven unclear points;
all were fixed in the skill and template before the version commit:

1. A design ending at step 1 with no actors conflicted with "finished when step 9 is confirmed": section 6 now ends
   it there, recorded under Decisions, later steps `not started`.
2. Whether confirming a step accepts its hypotheses: they stay open until settled, and the agent says which remain.
3. Delegation scope: steps outside the named scope keep the default; ask when unclear.
4. A revised earlier step returns to `draft`.
5. When to create the document: when the user agrees to start and the first result is written.
6. Template: "Next questions" (up to three), and a context drawing slot in step 1.
7. A missing source: steps 1 and 2 proceed; before step 3 the user chooses to resolve the source or continue with the
   unverified facts recorded as open questions.

## Round 2 (after the fixes)

<filled in below once the second run finishes>

## Not verified

- Live plugin discovery as `/cicada:actor-design`: `claude plugin validate` on the worktree passed (one public-directory
  icon warning, as before), but the plugin was not installed into a Claude Code session, so discovery and loading of
  the second skill are unverified.
- The resolver run from an installed sibling folder in a real consumer project: covered structurally by
  `test_both_unpacked_skills_sit_side_by_side_and_the_sibling_resolver_runs` (an unpacked format-2 archive), not in a
  live agent session.
- Cases not run by an agent: timeout-story, existing-file-protection, no-repository, old-source, mixed-source,
  mismatched-sibling, archive-layout. Other models and harnesses were not tried.
