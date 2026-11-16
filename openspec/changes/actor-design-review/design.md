# Design

The owner delegated remaining design decisions and approvals on 2026-11-16 and selected Opus 5.5 as reviewer.
One behavior: review an existing consumer actor design or implementation through the actor-design skill. Four
implementation tasks plus finishing suffice. This extends the method under ADRs 0041/0050/0051, not runtime
semantics or archive layout; no new ADR is needed. Knowledge stays in the resolved source's guide.

## Mode, scope and evidence

1. Extend skills/actor-design/SKILL.md's description and mode selection. Explicit review/critique of an existing
   artifact or code selects review; designing a new system retains the nine-step interview. If intent is ambiguous,
   ask one question while inspecting supplied evidence. Do not force a review request through all design steps.
   A combined request reviews the existing system first, then performs only the separately requested design work.
   Update cicada's entry point to name design and review. Neither mode grants automatic fixes or publication.
2. Reuse the sibling resolver and all source/status/fallback rules from actor-design-skill. Separate consumer code
   identity (paths, revision and dirty/unversioned state) from Cicada dependency identity and installed skill age.
   Record requested scope, selected stories/actors/configuration and exclusions. Read consumer instructions and
   existing design before inspection. The bundled read-only resolver (including locked/offline Cargo metadata) and
   read-only inspection are allowed; do not run consumer builds, tests or project scripts just to reconstruct it;
   separately requested verification retains its normal effects and must be reported accurately.
3. Review has four phases: Scope and evidence; Reconstruct; Check; Report. A review request authorizes completion
   of those phases without design-mode confirmation after each. Show the reconstructed map before findings, invite
   corrections and continue when evidence suffices. Ask at most three focused questions per turn when missing intent would
   change a conclusion; continue independent checks. Unanswered questions stay unknown, never inferred approval.
   Explicit consumer delegation permits stated choices within scope, not invented facts about existing code.
4. Tag claims as observed (code/test inspected), documented intent, inference or unknown; distinguish tests read
   from tests run. Cite consumer file plus symbol/heading and line where available, snapshot identity and relevant
   resolved guide section. For pasted material use section/quoted excerpt, not a fictional path. Where intent and
   implementation disagree, show both. A missing document alone does not prove missing runtime behavior.

## Reconstruction and judgment

5. Before judging, draw an actor/message/watch map from evidence, success/failure sequences for selected stories,
   and lifecycle diagrams where states/lives matter. Distinguish external systems, host calls and committed actor
   messages; mark inferred/unknown edges in text and the diagram legend. Missing evidence can yield a partial map.
   For a tiny or non-actor scope, show its actual context rather than inventing actors or watches. Do not make a
   complete-codebase claim from representative stories. Mermaid is the portable stored form, as in design mode.
6. Use the resolved guide's ownership, identities/lifecycle, state/bounds, protocols, coordination/time, recovery
   and outside-effects advice; keep source routes in the existing explicit Source routes section. For each area
   report supported, finding, unknown or not applicable with evidence/reason. Do not copy the research checklist
   into the skill or import another framework's semantics. Proposals and future pinning are not implemented APIs.
   Route state/bounds to Choose actors and identities; no new heading. Missing headings use source fallback.
7. Findings require a concrete violated requirement or supported failure risk, its preconditions and a short trace:
   initial state, messages/failure interleaving and bad outcome. Include id R-001, R-002, etc., priority, evidence, consequence,
   smallest useful correction and a suggested test/trace that would check it. High means credible loss of data or
   isolation, irreversible duplicate effect, or accepted work stuck without recovery; medium means incorrect but
   recoverable behavior or unbounded resource risk; low means a bounded maintainability/protocol ambiguity issue.
   Explain impact in the actual scope; severity is not a confidence score. Deduplicate findings sharing one cause.
8. Unknown requirements/evidence go in Open questions with conditional risks, not asserted defects. A documented
   tradeoff satisfying requirements goes in Accepted tradeoffs, not a finding merely because another decomposition
   is possible. A claimed tradeoff that violates a stated invariant is still a finding with evidence. Report no
   findings when supported; never manufacture a quota or treat a clean subset as proof of system correctness.

## Report, continuation and distribution

9. Default to docs/design/actors-review.md, honoring user paths/repository conventions. This separate artifact
   preserves docs/design/actors.md and code. Resume an existing report only when its Scope/Progress match the
   requested system and review scope (revision changes are allowed); unrelated or uncertain identity is a collision
   requiring a new destination or clarification. Prohibited/no repository writes keep the report in chat, unsaved. Define the
   report skeleton directly in SKILL.md: Scope and sources; Progress; Reconstructed system; Coverage; Findings;
   Accepted tradeoffs; Open questions; Checks and limits; Next actions. No new packaged template/member is needed.
10. Progress records phase, pending questions and evidence snapshot. Resume by reading the report and rechecking
    inputs/source selection. A changed snapshot marks affected observations/findings stale until rechecked; preserve
    user annotations, earlier ids and dispositions. Reuse an id for the same cause/location; never renumber or reuse
    retired ids, and append the next unused number for new findings. Mark a finding resolved
    only with new evidence, retaining its prior trace. Keep accepted/deferred separate from resolved. If impact
    cannot be localized, mark the whole review stale. A rerun does not erase earlier limitations or imply tests ran.
11. Extend examples/tests/skill/actor-design-evals.json with review cases and small test-only inputs under
    examples/tests/skill/review-fixtures/. Include sound alternative boundaries, duplicate outside effect on replay,
    stale reply after key reuse, logical wait cycle, missing external guarantees, docs/code disagreement, old/mixed
    source, partial scope, mode selection, report collision and changed-source resumption. Specify expected evidence,
    failure traces and forbidden claims; do not prescribe exact prose or one actor decomposition. skill.rs checks
    review-case shape/coverage, referenced fixture existence/safe relative paths, report-section instructions and
    source routes. Fixtures are test-only, not archive members. Behavioral review runs representative cases.

## Ownership and checks

12. No archive members, format marker or plugin manifests change beyond ordinary release versions. Existing package
    tests must prove updated SKILL.md is shipped with both unchanged roots/provenance rules. Keep design-mode evals
    intact. Verify skill/routes/package checks and record representative review transcripts and unverified behavior.
13. Handoff asks docs owner for mode selection/output paths in docs/how-to/install.md, docs/agents.md and
    docs/README.md; no new guide heading, Rust doc block, G/S row, generated API or API behavior is required. Preserve
    all existing links. Unexpected protected-prose test failures block landing until exact owner fixes integrate.
    Builder owns skill, test fixtures/checks, release pins/changelog and docs-handoff.md. Finish with strict OpenSpec,
    docs/doctests and the full committed-tree gate including PostgreSQL/Docker; no paid model/vendor gate.
