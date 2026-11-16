## MODIFIED Requirements

### Requirement: A portable actor-design skill guides consumer design work

Cicada SHALL provide skills/actor-design/SKILL.md and assets/design-template.md. Its version SHALL match the workspace
and sibling cicada skill. It SHALL hold the interactive design and existing-system review methods and route knowledge
to the resolved source's design guide. The existing cicada skill SHALL retain implementation, testing and operations,
and its entry points SHALL distinguish actor-design's design and review modes from coding.

#### Scenario: Choose the appropriate skill

- **WHEN** a consumer asks to work out actor boundaries, responsibilities or protocols for a new system
- **THEN** actor-design offers its step-by-step design method
- **WHEN** a consumer explicitly asks to review an existing design or implementation
- **THEN** actor-design offers its review method without imposing the design interview
- **AND** cicada's entry points name both methods by skill name rather than a sibling source path
- **AND** the plugin discovers it as /cicada:actor-design without manifest component overrides; this is a manual
  harness check, not a CI guarantee, and unavailable harness evidence is reported
- **AND** neither skill imposes Cicada's contributor loops on a consumer

## ADDED Requirements

### Requirement: The actor-design skill reviews existing designs and implementations

The actor-design skill SHALL offer review mode for explicit review of existing consumer designs or code. It SHALL
retain the design interview for new systems and reuse the existing resolved-source discovery and fallback rules.
Review SHALL have Scope and evidence, Reconstruct, Check and Report phases, without imposing design-mode checkpoints.

#### Scenario: Select and bound a review

- **WHEN** a consumer asks for a review of an existing design or implementation
- **THEN** the skill records the requested actors/stories/configuration, evidence identity and exclusions
- **AND** it completes the review within that scope without demanding confirmation after every phase
- **AND** ambiguous mode selection prompts a focused question while independent inspection may continue
- **AND** a combined review/design request reviews existing evidence before the separately requested design work
- **AND** the bundled read-only resolver, including locked/offline Cargo metadata, and read-only inspection are allowed
- **AND** review alone does not authorize code fixes, consumer-project builds/tests/scripts, dependency changes or publication

#### Scenario: Resolve the evidence source

- **WHEN** a review relies on Cicada semantics
- **THEN** it uses the consumer's resolved dependency guide/code and records its identity separately from consumer
  revision, local modifications and installed skill metadata
- **AND** mixed, missing, older and unavailable sources retain the predecessor's disclosure and fallback behavior
- **AND** proposals, latest main or absent documents are not proof of the consumer's runtime behavior

### Requirement: A review reconstructs the system before judging it

The review SHALL show evidence-based actor/message/watch maps, sequences for selected stories and lifecycle diagrams
where applicable before its findings. It SHALL distinguish observed behavior, documented intent, inference and
unknowns, including host calls, committed effects and outside systems.

#### Scenario: Reconstruct from code and documents

- **WHEN** the consumer supplies source, a design or both
- **THEN** claims cite file/symbol/heading and line where available, or a supplied excerpt when no path exists
- **AND** the map and legend identify inferred or unknown relationships and the report distinguishes tests read/run
- **AND** code/document disagreement is shown with evidence for both
- **AND** a partial or non-actor scope has an honest context diagram rather than invented actors or complete coverage

#### Scenario: Intent is missing

- **WHEN** missing requirements would change a conclusion
- **THEN** the skill asks at most three focused questions per turn while continuing independent checks
- **AND** unanswered questions and conditional risks remain labelled, without treating silence as confirmation
- **AND** delegated decisions do not become invented facts about the existing system

### Requirement: Findings are actionable and distinguish risks from tradeoffs

The review SHALL assess ownership, identity/lifecycle, state/bounds, protocols, coordination/time, recovery and outside
effects through the resolved guide; state/bounds SHALL route to Choose actors and identities. Each area SHALL be supported, a finding, unknown or not applicable with reasons.
Findings SHALL name a supported violation/risk, preconditions, evidence and a concrete counterexample trace.

#### Scenario: Report a supported defect

- **WHEN** evidence supports a failure risk or violation of the consumer's requirements
- **THEN** the finding includes a stable id (R-001, R-002 and so on), priority, locations, initial state/interleaving/bad outcome, consequence,
  smallest useful correction and suggested test or trace
- **AND** high priority means credible data/isolation loss, irreversible duplication or accepted work without
  recovery; medium means recoverable incorrect behavior or unbounded resource risk; low means bounded
  maintainability or protocol ambiguity
- **AND** priority is explained by actual impact, not confused with confidence, and common causes are deduplicated

#### Scenario: Sound or incomplete evidence

- **WHEN** a documented tradeoff meets requirements despite another valid decomposition
- **THEN** it is recorded as an accepted tradeoff, not manufactured into a defect
- **WHEN** evidence or requirements are insufficient
- **THEN** the report records open questions and conditional risks rather than asserting unproven defects
- **AND** a stated tradeoff that demonstrably violates an invariant remains a finding
- **AND** no findings is valid and does not claim proof of correctness outside the reviewed scope

### Requirement: Review reports preserve the design and can be resumed

The default report SHALL be docs/design/actors-review.md, subject to user paths and repository conventions. It SHALL
preserve the original design/code and existing user content. Its sections SHALL be Scope and sources, Progress,
Reconstructed system, Coverage, Findings, Accepted tradeoffs, Open questions, Checks and limits, and Next actions.
The report skeleton SHALL be defined in SKILL.md without extending the archive member set.

#### Scenario: Save a report safely

- **WHEN** a report destination already exists
- **THEN** the skill resumes it when Scope/Progress identify the same system and review scope, allowing changed revisions
- **AND** unrelated or uncertain identity is a collision requiring a different destination or clarification
- **AND** related user content is preserved
- **AND** it does not overwrite the design-mode document or implement proposed corrections
- **WHEN** repository writes are unavailable or prohibited
- **THEN** it provides the same report in chat and says it was not saved

#### Scenario: Resume after an input changes

- **WHEN** resuming a saved review
- **THEN** the skill reads its phase/questions/evidence snapshot and rechecks consumer and dependency identity
- **AND** changed evidence marks affected observations/findings stale, or the whole review if impact cannot be localized
- **AND** finding ids, prior traces, user annotations and dispositions survive updates
- **AND** the same cause/location reuses its id, new findings append the next unused number, and retired ids are never reused
- **AND** resolved requires new verifying evidence; acceptance or deferral alone does not resolve a defect
- **AND** tests not run and other remaining limits stay explicit

### Requirement: Review behavior is evaluated without changing distribution

Review cases SHALL extend examples/tests/skill/actor-design-evals.json, with small test-only inputs under
examples/tests/skill/review-fixtures/. Existing design cases and both-skill archive/plugin behavior SHALL be preserved.
examples/tests/skill.rs SHALL check review-case shape/coverage, fixture existence and safe relative fixture paths,
report-section instructions and source routes. Fixtures SHALL remain outside release archives. Behavioral review
SHALL assess representative outputs.

#### Scenario: Review evaluation coverage

- **WHEN** the implementation is checked
- **THEN** cases include a sound alternative design, replayed outside effect, stale reply/key reuse, wait cycle,
  unknown external guarantees, documentation/code conflict, old/mixed sources, partial scope, mode selection,
  destination collision and changed-source resumption
- **AND** expectations require grounded findings/traces, useful diagrams and preserved content, not exact wording
- **AND** the review records actual behavioral checks and limits without requiring a model service in the gate

#### Scenario: Keep packaging and guidance consistent

- **WHEN** review mode is released
- **THEN** existing packaging checks verify the updated skill bytes within the unchanged member/provenance contract
- **AND** the builder hands mode/output-path guidance to the docs owner for install and navigation entry points
- **AND** protected-prose failures require exact docs-owner fixes rather than skipped checks
