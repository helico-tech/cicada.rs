## ADDED Requirements

### Requirement: A portable actor-design skill guides consumer design work

Cicada SHALL provide skills/actor-design/SKILL.md and assets/design-template.md. Its version SHALL match the workspace
and sibling cicada skill. It SHALL hold the interactive method and route knowledge to the resolved source's design
guide. The existing cicada skill SHALL retain implementation, testing and operations; review mode SHALL remain out
of scope until its separate change.

#### Scenario: Choose the appropriate skill

- **WHEN** a consumer asks to work out actor boundaries, responsibilities or protocols
- **THEN** actor-design offers its step-by-step method and cicada's entry points distinguish that method from coding
- **AND** the plugin discovers it as /cicada:actor-design without manifest component overrides; this is a manual
  harness check, not a CI guarantee, and unavailable harness evidence is reported
- **AND** neither skill imposes Cicada's contributor loops on a consumer

### Requirement: The design method uses the existing resolver without copying it

The new skill SHALL locate the sibling cicada resolver from its installed folder, quote the actual path and run
from the consumer project. It SHALL preserve the existing locked/offline, target/features and source-selection
rules. It SHALL distinguish the sibling resolver's reported provenance from its own installed metadata.

#### Scenario: Installed siblings and source identity

- **WHEN** both skills are installed from an archive or plugin
- **THEN** the same sibling resolver is used, without a second script or harness-specific path variable
- **AND** differing sibling versions or available SOURCE.json identities are disclosed rather than treated as a match
- **AND** the resolved dependency guides/code remain authoritative over either skill's installation tree

#### Scenario: Missing sibling, older source or ambiguity

- **WHEN** the sibling is missing/mismatched, discovery fails, sources are mixed or design.md is absent
- **THEN** the limitation is recorded and the existing manual discovery or verified older source routes are used
- **AND** source-dependent decisions wait for resolution where necessary while generic problem exploration may proceed
- **AND** no implicit fetch, install, dependency change or fallback to latest-main guidance occurs
- **AND** self, none and no-docs keep the existing resolver's meanings

### Requirement: Design progresses through explicit, visual checkpoints

By default the skill SHALL work through the nine steps one at a time, ask at most three focused questions per turn,
and obtain confirmation of a step's written result and drawing before advancing. It SHALL honor explicit consumer
delegation within scope, recording delegated choices and reasons instead of claiming user confirmation.

#### Scenario: Build from the user's concrete story

- **WHEN** a user describes a problem
- **THEN** the method covers promise, domain story, invariant owners, identity/lifecycle, responsibility cards,
  protocols, coordination/time, outside effects and checking/simplification
- **AND** template and Progress use the exact nine step names/order from the landed guide, checked by skill.rs
- **AND** it uses the user's vocabulary and labels hypotheses and missing requirements as open questions
- **AND** rejecting actors in favor of a simpler design is a valid result

#### Scenario: Confirmation and explicit delegation

- **WHEN** the current step's result is drafted without decision delegation
- **THEN** it stays draft until the user confirms it, and silence does not advance the method
- **WHEN** the consumer explicitly delegates decisions
- **THEN** the agent may decide within that scope, recording reasons and remaining uncertainty
- **AND** that delegation does not authorize unrelated implementation, commits or deployment

#### Scenario: Diagrams evolve without invented actors

- **WHEN** only the domain story is known
- **THEN** the drawing shows domain context and unknowns without asserting final actor boundaries
- **WHEN** boundaries and protocols become known
- **THEN** diagrams show actors/messages/watches, relevant lifecycle states and per-story sequences
- **AND** by step 7 each story covers timeout, repeat, reorder, cancellation and dependency failure, or records why
  a branch is inapplicable, distinguishing host calls and atomic commits

### Requirement: The design document preserves decisions and supports resumption

The default saved artifact SHALL be docs/design/actors.md in the consumer repository, overridden by the user's path
or repository convention. It SHALL contain source/scope, progress, all nine step sections, decisions and open
questions. Existing user work SHALL be inspected and preserved, not replaced by a template.

#### Scenario: Save or choose another destination

- **WHEN** no destination already exists and the task permits writing
- **THEN** the skill starts from its bundled template and saves proposed results as drafts at the selected path
- **WHEN** the destination belongs to another design
- **THEN** it asks for a new name rather than overwriting it
- **WHEN** there is no writable repository or writing is prohibited
- **THEN** it presents the same artifact in chat and reports that it was not saved

#### Scenario: Resume or revise an earlier result

- **WHEN** work resumes
- **THEN** the skill reads the artifact, verifies source selection and continues at the recorded step/next question
- **AND** it does not restart the interview or label draft content confirmed
- **WHEN** a requirement changes an earlier result
- **THEN** affected later sections are marked for review while preserving their content and unaffected decisions
- **AND** it never auto-commits the design or records credentials in its source context

### Requirement: One versioned archive carries both skills while preserving legacy archives

Format 2 of cicada-skill-vX.Y.Z.tar.gz SHALL retain all cicada/ members and add actor-design/SKILL.md,
actor-design/assets/design-template.md and actor-design/SOURCE.json with required directories. Both source manifests
SHALL identify the same tagged repository, commit and version. Content SHALL come from explicit regular tagged
blobs, with deterministic members/modes/timestamps and safe paths under the two allowed roots.

#### Scenario: Select layout from the immutable source

- **WHEN** the tag contains scripts/skill-archive-format with exactly `2` and a newline
- **THEN** both complete skills and their matching metadata are required; absence cannot downgrade the layout
- **WHEN** the marker and actor-design tree are both absent
- **THEN** packaging reproduces the existing single-skill layout and bytes, including the fixed historical fixture
- **WHEN** the marker is unknown/malformed or absent alongside an actor-design tree
- **THEN** packaging fails before writing an artifact

#### Scenario: Invalid or non-deterministic content is refused

- **WHEN** a required file is missing, a skill version differs, a required source is a symlink, or the archive has
  unsafe, duplicate, extra or wrong-type members
- **THEN** packaging fails rather than publishing a partial or different layout
- **AND** unrelated working-tree edits never change the bytes packaged from the same tag
- **AND** single-root conformance-fixture packaging keeps its previous behavior

#### Scenario: Installation and upgrade compatibility

- **WHEN** consumers upgrade to the two-skill archive
- **THEN** existing cicada/ paths remain valid and documentation stages, compares and installs both sibling folders
- **AND** local edits are preserved for a deliberate update; no automatic replacement or dependency upgrade occurs
- **AND** a consumer may still install cicada alone, but actor-design's resolver dependency is stated

### Requirement: Release verification understands both layouts and all provenance files

Current release preflight SHALL require format marker 2, both skill versions and the plugin version to match the
workspace/release. The driver SHALL select layout from its pinned commit and validate every SOURCE.json and tagged
file. Legacy recovery, immutable tags, existing release digests and fixture-archive checks SHALL remain intact.

#### Scenario: New release preflight or packaging fails

- **WHEN** marker, skill, plugin version or either source manifest is invalid
- **THEN** the release attempt fails at the applicable existing preflight/package stage before publishing a Release
- **AND** already-pushed immutable tags or saved digests are never rewritten as a recovery shortcut

#### Scenario: Recover a legacy release

- **WHEN** the driver resumes a pinned legacy commit without the new marker or actor-design tree
- **THEN** it verifies the legacy member set and preserves its existing bytes/digests, independently of current cwd
- **AND** format-2 verification recognizes both SOURCE.json files as files rather than directories

#### Scenario: Resume format 2 after a driver update

- **WHEN** a format-2 release has saved archive digests and the caller's release driver is then updated
- **THEN** the updated driver is refused before publishing anything, leaving the saved digests unchanged
- **AND** resumption runs the pinned commit's driver, which verifies both source manifests and tagged members against
  the pinned commit and saved digests
- **AND** a mismatch fails instead of rewriting digests or substituting current working-tree content

### Requirement: Structural checks and behavioral review cover the design method

Tests SHALL check both skill versions/names, explicit source routes, the template sections, evaluation coverage and
archive/release behavior. Staged cases SHALL live in examples/tests/skill/actor-design-evals.json. Behavioral review SHALL exercise representative staged conversations and judge decisions,
diagrams and preserved work, without requiring a model service in the repository gate.

#### Scenario: Route and template checks

- **WHEN** a required design heading is missing, a source route breaks, a template section is absent or a consumer
  route points into contributor-only material
- **THEN** structural checks identify the missing entry or forbidden route
- **AND** output artifact/template paths are distinguished from routes beneath the resolved Cicada source
- **AND** existing cicada route coverage is retained

#### Scenario: Review the actual interaction

- **WHEN** the implementation is reviewed
- **THEN** cases cover a fresh design, a non-actor problem, a failure story, confirmation/delegation, resume/revision,
  preservation of existing text, old/mixed/missing sources/sibling and both install layouts
- **AND** review checks no premature advancement, labelled assumptions and evidence, meaningful Mermaid diagrams
  and preserved user content, rather than matching exact wording
- **AND** the handoff names installation/update, entry-point and release-guide changes for the docs owner
