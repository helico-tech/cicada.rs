# Behavioral review of actor-design's review mode (builder's evidence)

What was exercised while building, and what remains unverified. This is evidence for the reviewer, not a claim of
model compliance; CI checks only the structure of the skill, its staged cases and their fixtures.

## How

Three fresh subagents (Claude Opus 5.5, the builder's model family) each read `skills/actor-design/SKILL.md` (at
commit 7376818, before the version commit, so they saw version 0.19.3) and the sibling `cicada` skill, and played the
agent for staged cases from `examples/tests/skill/actor-design-evals.json`, one user turn at a time. They saw the
setup and the turns, not `expect` or `avoid`. Each case ran in its own folder under `target/review-runs/` with the
case's files from `examples/tests/skill/review-fixtures/`. The resolver was not run (the folders hold no Cargo
project): every case assumed a report of status `ok`, cicada 0.19.3 from crates.io, `skill_source: null` (plugin
layout), with the root set to the builder's worktree, so the agents read the real design guide. Git was refused in
those folders by the session's worktree isolation, so every project was an "unversioned fixture snapshot"; the resume
case simulated its two revisions with a `REVISION` file. The builder judged the saved reports and chat replies
against `expect` and `avoid`.

## Results

| Case | Result |
| --- | --- |
| review-mode-selection | Turn 1 read the three files, wrote nothing, and asked one question: design something new or review what exists. Turn 2 ran all four phases without waiting between them and saved `docs/design/actors-review.md`. |
| review-sound-alternative | Map, two story sequences with expiry and late-reply branches, a line state diagram and a design-against-code table before any finding. Cart plus checkout recorded as an accepted tradeoff with its documented reason, explicitly "not a finding". It found one real medium defect in the fixture (below). |
| review-duplicate-effect | R-001 high with a trace (provider charges, response lost, handler `Err`, retry reruns the same message and charges again; also a lost claim); correction: the message id as `Idempotency-Key`; check: a TestActors test with a recording gateway double. R-002 high: an absent month ledger rejects the commit after the charge, not retried. R-003 medium: a decline has no reply. Checks and limits: nothing ran, no tests exist. No claim that Cicada makes the call once. |
| review-stale-reply | R-001 high: close and reopen `lobby` as private while a `Check` waits in the moderator's queue; the stale `Approved` adds the user to the new life. Correction: a pending request id per life, or a new key per life; check: a TestActors trace. R-002 low: a reply to a closed room rejects the moderator's commit. |
| review-wait-cycle | R-001 high: Hold and Confirm wait for each other with no deadline, the amount already deducted. R-002 high: an insufficient balance gets no reply. R-003 medium: holds are never settled or released. No lock or blocking-call framing. |
| review-unknown-guarantees | No findings asserted. Postbird's handling of `client_reference` is open question Q1 with the conditional risk of a second email. After the user's "decide whatever you need", it recorded the answer, kept Postbird's behaviour unknown ("a delegation cannot supply a fact about Postbird"), and made two labelled delegated decisions with reasons and what stays uncertain. |
| review-docs-code-conflict | R-001 medium: holds never expire because the design's 10-minute timer is missing from the code, with the documented rule and the observed code side by side; Q1 asks whether the timer is still the rule; the silently ignored `Pay` against "refused" is raised. The design document was left unchanged. |
| review-changed-source-resume | Read the saved report, recorded the new snapshot and the earlier one, and, with no diff readable, marked the whole review stale and rechecked every claim. Round 1 marked R-001 resolved on the key (`payment.rs:46-47`) although only comments state the provider's deduplication; review round 1 rejected that (see Round 2). R-002 still open, line moved, Dana's note kept; R-003 still withdrawn; the new receipt email became R-004 (high). Only the changed sections were edited; "Ran: nothing, in either run". |
| review-report-collision | Saw the billing team's notes ("please keep"), changed nothing, created no file, asked for another path with a suggested name, and showed the room review in chat marked as not saved. |
| review-partial-scope | Scope and exclusions recorded; shipping opened only for the type it receives and listed as such; no findings in scope, stated as not covering shipping or inventory; who creates `shipping/<order>` and who refunds a late payment are open questions. It noticed a keyless courier call in excluded code and did not report it. |
| review-non-actor-scope | Drew the real context (admin, `save`, the settings table, readers marked `?`), said the scope holds no actors, and marked the areas not applicable with reasons; no actors invented. |

Not run by an agent: review-combined-request, review-old-source, review-mixed-source, review-no-repository. Other
models and harnesses were not tried. The sound-alternative and mode-selection runs were one agent on the same files,
so the second is not independent.

## Fixture corrected

The sound-alternative fixture was meant to have no defect, but checkout never settled its reservations, so a SKU's
capped reservation map filled for good. The agent's finding was right; the fixture's cart now sends `Confirm` for each
line at checkout and stock forgets the entry while keeping the count taken. It was re-run in Round 2.

## Skill clarified after the runs

The agents named points where the skill left a judgement call. These were clarified within the design:

1. A resumed review recorded as done goes back to Check for what is stale, and Progress records what was stale and
   what was checked again; the report skeleton keeps earlier snapshots.
2. Without git, the revision is recorded as unknown and local changes as not checked; no readable diff marks the whole
   review stale.
3. A code comment counts as documented, and a comment about an outside system documents intent, not that system's
   behaviour.
4. A carried-over finding keeps its priority unless the evidence changed; a new grade and its reason go under History.
5. On a collision the review may be shown in chat meanwhile, marked as not saved.
6. A file opened only at the boundary is listed as opened, not reviewed; a risk noticed outside the scope is one line
   in Checks and limits, not a finding.
7. An area with a finding and unknown parts is a finding that names the unknown; a focused question picks the stories
   and leads the summary while all seven areas still get verdicts; a review against a design may add a table of its
   statements beside the code.
8. The report's Open questions may hold more than the three asked in one turn; a delegated choice is written under
   the question it settles, with its reason and what stays uncertain (the report has no Decisions section).
9. The Recovery area also routes to `docs/architecture.md#turn-semantics`, which states that a send to an absent
   actor rejects the commit and that rejected commits are not retried; two agents needed it and reached it only
   through links.

Judgement calls left as they are: where the line runs between a supported risk and an open question when a
precondition depends on a sender outside the scope (the agents chose questions); how literally to apply
"irreversible duplicate outside effect" to a duplicate receipt email (graded high in both resume runs); and the
Ownership verdict of a scope without actors (the agent chose supported). Settled in review round 1: `resolved` may not
rest on an unverified comment about an outside system (Round 2).

## Round 2 (review round 1: resolution needs evidence)

The code review found that the resume case expected R-001 resolved, while `resume/after/src/gateway.rs` holds only a
trait and comments about the provider; the skill says such a comment is documented intent, not the provider's
behaviour, and `resolved` needs evidence that the cause is gone. The case now expects R-001 to stay open, with the
observed change under History and an open question for the provider's deduplication, and avoids resolving it on a
comment. Dispositions in section 8 now say so in one sentence: a change whose effect rests on an outside system's
unverified behaviour records the change, keeps the finding open and asks for the evidence.

Two fresh subagents ran against the final skill (all clarifications above plus that sentence; version 0.19.4, the
resolver report assumed as before at 0.19.4; git refused again, revisions simulated with `REVISION`):

| Case | Result |
| --- | --- |
| review-changed-source-resume | R-001 stays `open` and high; History records the key at `payment.rs:46-47` and that its effect rests on comments only (`gateway.rs:1, 6-7`); Q1 asks what shows that the client sends `Idempotency-Key` and the provider returns the first receipt for 24 hours; Correction and Check now ask for that evidence. Earlier trace kept. R-002 open with Dana's note and its moved line; R-003 still withdrawn; the receipt email is new R-004 (high). Earlier snapshot kept in Progress, with what was stale and checked again. It also corrected the old report's unproven "skills match" to "not comparable". Ran nothing. |
| review-sound-alternative (corrected fixture) | No findings; cart plus checkout recorded as an accepted tradeoff (a second documented decision, stock relying on the cart's timer, also listed); a ten-row design-against-code table all agreeing; four open questions, among them forged replies if outside callers can send `CartMsg` (conditional high) and who creates and deletes carts and stock. |

Points the agents still raised, left as judgement calls: whether a one-turn review must save after every phase;
whether the old Correction text of a partly addressed finding is replaced or kept; `supported` against `unknown` for
an area that holds under an open question; whether a plain documented decision belongs under Accepted tradeoffs.

## Not verified

- Live plugin discovery as `/cicada:actor-design` and the resolver run from an installed sibling in a real consumer
  project; covered structurally by the existing packaging tests.
- Behaviour after the clarifications above, except for the two Round 2 cases.
- The four cases not run, other models and harnesses.
