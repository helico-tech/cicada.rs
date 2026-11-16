# Docs handoff for actor-design-review

Version: 0.19.4. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The consumer skill `actor-design` gains a review mode. No Rust API, wire format, contract, schema, guarantee, archive
member or plugin manifest change (beyond the release versions).

- **Mode selection** (`skills/actor-design/SKILL.md`, `Choose the mode`). Design mode, the nine-step interview, stays
  for a new system. Review mode is chosen when the user explicitly asks to review, critique or check an existing design
  document or codebase. An unclear request ("look at our actors") gets one question while the agent already reads the
  supplied files; a combined request ("review what we have, then design refunds") reviews first, then designs only
  what was asked, in design mode and its own document. Neither mode fixes code, changes dependencies, commits or
  publishes.
- **Review method** (section 7). Four phases: Scope and evidence, Reconstruct, Check, Report. An explicit review
  request authorizes all four without design mode's confirmation after each step. The agent reads the consumer's own
  instructions and existing design first; records the requested scope and exclusions; keeps the consumer's code
  revision and local changes, the resolved Cicada source and the installed skill versions apart; and only reads
  (files, read-only git, the bundled resolver). It runs no consumer builds, tests or scripts for a review. It draws an
  actor/message/watch map, a sequence per selected story with failure branches and state diagrams before judging,
  marks inferred and unknown edges, and tags claims as observed, documented, inferred or unknown. It checks seven
  areas (ownership, identities and lifecycle, state and bounds, protocols, coordination and time, recovery, outside
  effects) against the resolved design guide, each `supported`, `finding`, `unknown` or `not applicable`. Findings
  carry an id (R-001 and on), a priority by impact (high, medium, low as defined in the skill), evidence, a
  counterexample trace, the smallest useful correction and a test or trace that would check it. Missing requirements
  become open questions with conditional risks (at most three questions per turn); documented tradeoffs that meet the
  requirements stay accepted; no findings is a valid result.
- **Report** (section 8). Default `docs/design/actors-review.md`, a separate file that never replaces the design
  document `docs/design/actors.md`; a path the user or the repository names wins. Sections: Scope and sources,
  Progress, Reconstructed system, Coverage, Findings, Accepted tradeoffs, Open questions, Checks and limits, Next
  actions. A report of the same system and scope is resumed (a changed revision is fine); another system's file is a
  collision and the agent asks. On resume, findings that rest on changed evidence become stale until rechecked; ids
  are never renumbered or reused; `resolved` needs new evidence, `accepted` and `deferred` are not resolved; the
  user's notes stay. Without a writable repository the report is shown in chat and marked unsaved.
- **Entry points.** The `cicada` skill's description and introduction name both modes of `actor-design`.
- **Tests and evidence.** `examples/tests/skill.rs`: `the_design_evaluation_cases_are_complete` (now also review cases,
  their `mode` and safe, existing fixture paths), `the_review_method_defines_its_phases_areas_and_report`,
  `an_unsafe_fixture_or_a_missing_review_part_is_named`. `examples/tests/skill/actor-design-evals.json` gains 15
  review cases; their test-only inputs are under `examples/tests/skill/review-fixtures/` (never packaged). The
  existing packaging tests ship the updated `SKILL.md` within the unchanged format-2 member set. Behavioral runs:
  `openspec/changes/actor-design-review/behavioral-review.md`.

This handoff builds on the one of 0.19.3 (`actor-design-skill`), whose install, update and entry-point text is not
in the protected docs yet. Where both touch the same place, write them together.

## 1. Install: what each skill does

**Where.** `docs/how-to/install.md`, `Install the coding-agent skill`. Keep the heading and its anchor.
**Text.** Where the two consumer skills are introduced (0.19.3 handoff, section 1), say that `actor-design` has two
modes: design, which works out a new design with the user step by step and saves `docs/design/actors.md`; and review,
which checks an existing design or implementation and saves `docs/design/actors-review.md`. Ask for a review in plain
words, such as "review our order actors"; an unclear request gets one question about the mode.
**Caveats.** A review reads only; it runs no builds or tests of the project and changes no code. Its findings rest on
the files it read and the Cicada source the project resolves, not on the whole system. Both output paths are defaults;
a path the user names or the repository's own rules win.

## 2. Update: saved reports

**Where.** `docs/how-to/install.md`, `Update the installed skill`. Keep the heading and its anchor.
**Text.** Add to the 0.19.3 caveat on saved design documents: review reports (`docs/design/actors-review.md`) are the
consumer's files too; updating the skills never touches them. A later review resumes a saved report and marks
findings stale when the code or the Cicada source changed.
**Caveats.** None beyond the above.

## 3. Entry points

**Where.** `docs/README.md`, `Work with an agent`; `docs/agents.md`, `Read only what the task needs` (row `Design actors
and protocols`).
**Text.** README: the line about `actor-design` (0.19.3 handoff, section 3) can read: designs a system with the user
step by step, or reviews an existing actor design or implementation, and saves the result as a Markdown document with
diagrams. Reading map: the design row can add that the skill's review mode checks an existing system against the same
guide's sections, with `docs/guide/design.md#recognize-design-mistakes` for the common mistakes.
**Caveats.** Neither mode is a substitute for the resolved source's guides; a review is not a proof of correctness.

## Blocking

None. No doctest, link, guarantee row or skill route needs protected prose.

## Status

- Sections 1 to 3 are proposed text; nothing protected was edited.
- Checks run: see the change's report and `behavioral-review.md`.
