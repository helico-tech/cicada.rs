# consumer-skill Specification

## Purpose
A skill for coding agents in projects that use Cicada. It routes the agent to the guides, how-tos, references and
examples of the Cicada source the project actually resolves, and it is packaged with each release.

## Requirements

### Requirement: Consumer agents use a portable skill backed by authoritative sources

Cicada SHALL maintain skills/cicada/SKILL.md and its source-discovery helper for agents working in consumer projects.
The entry SHALL identify its Cicada version, explain when it applies and route tasks to maintained guides, examples
and API sources. It SHALL not impose Cicada's contributor loops on a consumer or require a particular agent harness.
Detailed reusable guidance SHALL live in the canonical guides rather than a copied skill manual.

#### Scenario: A skill installed outside the Cicada repository

- **WHEN** an agent loads the skill from a consumer's project skill directory
- **THEN** it resolves routing paths beneath the identified Cicada source root, not beneath the skill installation
- **AND** installation does not require relative ../../docs links, a plugin or a connector

#### Scenario: Progressive knowledge loading

- **WHEN** a task concerns actor design, testing, flows, HTTP, workers, telemetry or deployment
- **THEN** the skill directs the agent to the relevant source documents and worked examples for that task
- **AND** it need not load all manuals or recreate implementation examples inside SKILL.md

#### Scenario: Consumer versus contributor

- **WHEN** Cicada is a dependency of another project
- **THEN** the consumer's own instructions and requested task govern its work, without mandatory Cicada change loops
- **WHEN** the verified Cicada core and tree are members of the selected workspace itself
- **THEN** the resolver reports self and the skill points to that repository's contributor instructions

### Requirement: Source discovery preserves the consumer's resolved dependency identity

The read-only helper SHALL use locked offline Cargo metadata for the selected manifest, target and feature settings,
with exact package names and returned manifest paths. It SHALL report its selection and provenance in structured JSON.
It SHALL not infer resolved versions from manifest ranges, shortened git hashes or guessed checkout directory names.
It SHALL not fetch, install tools, update dependencies, generate a lockfile or overwrite the installed skill.

#### Scenario: Locked git dependency

- **WHEN** a consumer has fetched a locked Cicada git dependency
- **THEN** the report includes each selected Cicada package, version, full Cargo source identity, selector, full
  revision and actual manifest path, and the verified documentation root where present
- **AND** the resolver runs cargo metadata with --locked, --offline and --filter-platform, not --no-deps
- **AND** a consumer named cicada-code-factory is not counted merely because its name starts with cicada

#### Scenario: Workspace and feature configuration

- **WHEN** dependencies are inherited from workspace declarations or selected by target/features
- **THEN** discovery uses the resolved graph for the reported selection, accepting explicit manifest/target/features
- **AND** the host triple is the default target and the report does not imply every target or feature was examined
- **AND** --no-default-features, --all-features and explicit --features selections are supported
- **AND** target-excluded packages left in metadata.packages are excluded using IDs from the filtered resolve graph

#### Scenario: Path and patch provenance

- **WHEN** a dependency resolves to a local path or a patched fork
- **THEN** the report retains its actual source/root and version, with HEAD and dirty state when available or
  unversioned otherwise
- **AND** a matching package version does not make it equivalent to the skill's upstream commit
- **AND** git sources also expose a checkout HEAD discrepancy or dirty working tree rather than hiding modifications

#### Scenario: Multiple versions or sources

- **WHEN** the selected graph contains Cicada crates from different versions or source groups
- **THEN** status is mixed and the report lists the groups instead of silently picking one
- **AND** the agent establishes which package/configuration the task concerns before making single-source assumptions
- **AND** it may continue inspection that does not depend on that choice

#### Scenario: A reshaped or vendored source

- **WHEN** a known Cicada package is resolved but the expected source/documentation root cannot be verified
- **THEN** the report retains its manifest/source identity with docs_available false and reports no-docs for a single group
- **AND** the skill directs the agent to that exact package's source or rustdoc rather than claiming no dependency exists

#### Scenario: No dependency or unavailable resolution

- **WHEN** metadata resolves but contains no selected Cicada package
- **THEN** status is none
- **WHEN** the lockfile or tools are missing, metadata fails, or a bounded subprocess times out
- **THEN** status is unavailable with the reason and a concise diagnostic
- **AND** there is no retry without --locked/--offline, tool installation, lockfile change or automatic fetch
- **AND** a missing download may prompt a suggestion of cargo fetch --locked, not cargo update

#### Scenario: Missing rustup toolchain

- **WHEN** Cargo or rustc is a rustup proxy and its active toolchain is not installed
- **THEN** subprocesses have RUSTUP_AUTO_INSTALL=0 and discovery reports unavailable rather than installing it
- **AND** --offline is not treated as sufficient to suppress rustup's own installation behavior

#### Scenario: Resolver test input

- **WHEN** --metadata FILE supplies a saved metadata fixture
- **THEN** the helper can exercise package/group/root classification against temporary fixture trees without network
  access or a developer's private consumer checkout

### Requirement: Advice remains tied to verified source rather than the skill's age

The skill SHALL treat the resolved source's architecture, guides and code as authoritative for that dependency.
Version comparison SHALL be separate from revision identity and modified-source state. Missing documentation SHALL
be reported honestly and SHALL not justify invented APIs, blind upgrades or replacement with latest-main advice.

#### Scenario: Newer router with an older dependency

- **WHEN** the installed skill is newer than the consumer's resolved version
- **THEN** it reports the version relationship once and reads the older source's docs/code for the task
- **AND** it does not assume the old release has a downloadable skill artifact

#### Scenario: Equal version with a different commit

- **WHEN** the package and skill have the same numerical version but different revisions or modified source
- **THEN** the report does not claim an exact match
- **AND** version_relation is distinct from revision_match, and unknown provenance remains unknown

#### Scenario: A heading moved

- **WHEN** a routed file or heading is missing from the resolved source
- **THEN** the agent searches that source's README, guides and public code for the relevant functionality
- **AND** it calls the feature unsupported only when evidence supports that conclusion, otherwise unverified
- **AND** it does not use an OpenSpec proposal or latest-main document as proof that the consumer has the API

#### Scenario: Offline or missing helper prerequisites

- **WHEN** Python, a toolchain, matching documentation or network access is unavailable
- **THEN** the skill provides a manual path using lockfiles, manifests and verified local source with stated limits
- **AND** useful work supported by local evidence may continue without guessing Cargo cache paths
- **AND** any separately requested build/doc command is distinguished from the read-only discovery helper

#### Scenario: New integration

- **WHEN** the user asks to add Cicada to a new project
- **THEN** the skill recommends one verified published stable release for selected crates and explicit features for
  the chosen layer/store, unless the user chose another source
- **AND** an unreleased tag alone is not evidence of a published release
- **AND** offline use of a known skill release is described as a candidate, not a verification that it is latest
- **AND** explicit choices of a fork, path or commit are preserved; dependency edits/fetches stay within the user's task

### Requirement: The skill teaches effective use without overgeneralizing runtime rules

The skill SHALL highlight the core decision points and route to their version-specific detail. The core guide SHALL
include actor ownership/identity, serialization and state-size boundaries, message design and evolution. Existing
guides SHALL own durable flow, testing, HTTP, telemetry and operations advice; their Rust examples remain checked.

#### Scenario: Actor and external-effect design

- **WHEN** an agent designs an actor that updates state and calls an external service
- **THEN** guidance distinguishes atomic internal effects from at-least-once handler/external execution and uses
  the correct stable idempotency key for the operation, including a flow step's key when applicable
- **AND** actor identity and size follow the required consistency/serialization boundary rather than an arbitrary
  one-actor-for-everything or one-actor-per-request rule

#### Scenario: Timers, cancellation and flows

- **WHEN** a task needs durable waiting or replayable work
- **THEN** the skill routes to scheduled messages or flow sleep and the resolved version's replay/step/cancellation rules
- **AND** it does not universally prohibit bounded async work in a handler or claim blocking tasks stop on cancellation
- **AND** ready-made actors are considered when they fit, without forcing an extra crate for unrelated tasks

#### Scenario: State and rolling upgrades

- **WHEN** a change adds state fields or alters a contract
- **THEN** guidance separates reading old state with serde defaults from strict contract-hash compatibility
- **AND** deployment order follows the release's contract and migration requirements, not an unconditional rolling rule
- **AND** the skill does not authorize destructive contract replacement merely because the new Rust type decodes old data

#### Scenario: Testing and operations

- **WHEN** a consumer adds or changes actor behavior
- **THEN** the skill routes to virtual-time TestActors tests, contract snapshots and state samples as relevant
- **WHEN** the task exposes HTTP, operates workers or deploys nodes
- **THEN** it covers host-provided authentication, failure/uncertainty, lifecycle, telemetry and upgrade guidance
  using only APIs supported by the resolved source

### Requirement: Each new release publishes an identifiable skill artifact

The release workflow SHALL package the skill from the release tag's Git tree and attach cicada-skill-vX.Y.Z.tar.gz
after existing checks succeed. The archive SHALL contain cicada/SKILL.md, its resolver and SOURCE.json with repository,
tag, commit and version. Package provenance/version/path checks SHALL pass before GitHub Release creation.

#### Scenario: Exact release content

- **WHEN** packaging a tag whose workspace and skill versions match
- **THEN** the archive contains the skill from that commit and correct SOURCE.json, with portable relative paths
- **AND** repeated packaging yields identical bytes regardless of unrelated working-tree changes
- **AND** no absolute path, arbitrary working-tree file or copied documentation mirror is included

#### Scenario: Invalid release package

- **WHEN** the tag/skill/workspace versions disagree, expected files are missing or the package cannot be validated
- **THEN** packaging fails and the release workflow does not create a GitHub Release
- **AND** tests use a temporary Git repository/tag containing the new skill, not a historical tag that predates it

#### Scenario: Install and update

- **WHEN** a consumer installs a released skill
- **THEN** the documented process stages/examines the archive and installs it in the chosen agent's project skill folder
- **WHEN** an installed copy differs or a dependency changes
- **THEN** the process compares before replacing a customized copy and does not silently overwrite it or upgrade Cicada
- **AND** no background updater or moved release tag is required

### Requirement: Automated checks and behavioral review maintain the skill

CI SHALL check frontmatter/version, routing paths and anchors, maintained coverage of guides/operations/examples,
resolver fixtures and failures, and package layout/provenance/reproducibility. Guide code SHALL remain covered by
existing doctests. Representative consumer-task evaluations SHALL supplement these structural checks in review.

#### Scenario: A stale route or version

- **WHEN** a route breaks, a required guide/example entry loses coverage or the skill version differs from the workspace
- **THEN** the checks fail with the affected source or field
- **AND** runtime guidance does not route consumers to proposals or internal loop instructions

#### Scenario: Behavioral evaluation

- **WHEN** the skill is substantially changed
- **THEN** review exercises representative requests in temporary consumer projects, including fresh setup, duplicate
  external payment, flow cancellation, offline older versions, mixed sources, HTTP authentication and contract changes
- **AND** evaluations judge concrete decisions/artifacts instead of matching exact answer wording
- **AND** CI needs no model service and the project does not claim its checks prove complete or always-current knowledge

### Requirement: Consumer skills can be installed as one optional Claude Code plugin

The repository SHALL expose a marketplace named cicada with exactly one plugin named cicada, sourced from the
repository root using `./`. The plugin SHALL discover consumer skills from skills/ without copying them or listing
individual skills in either manifest. It SHALL NOT expose contributor loops or additional plugin components.
The release archive SHALL remain available to agents without Claude Code, with its existing layout and provenance.

#### Scenario: Install from the private repository

- **WHEN** a consumer with working Git read access adds helico-tech/cicada.rs as a marketplace and installs cicada@cicada
- **THEN** the plugin provides the existing skill as /cicada:cicada from skills/cicada
- **AND** no bundled command, hook, agent, server or executable is enabled
- **AND** repository copies in the plugin cache do not supersede the consumer's resolved Cicada source

#### Scenario: A later consumer skill

- **WHEN** a later change adds an ordinary skill directory under skills/ with SKILL.md
- **THEN** default plugin discovery includes it without another marketplace entry or manifest path
- **AND** the structural checks accept more than one consumer skill

#### Scenario: Portable helper invocation

- **WHEN** the skill is loaded from either the plugin cache or an unpacked archive
- **THEN** its common instructions use the actual, shell-quoted installed skill folder to locate the helper
- **AND** the helper runs from the consumer project without requiring a Claude-specific variable

### Requirement: Plugin versions track the workspace without claiming tag-exact provenance

The plugin manifest SHALL have the workspace version. The marketplace SHALL follow main and SHALL NOT pin a release
tag or repeat the plugin version. Skill checks and the release checker SHALL refuse missing, malformed or mismatched
plugin versions. Plugin installation/update SHALL NOT change the consumer's Cicada dependency.

#### Scenario: A stale or invalid manifest blocks release

- **WHEN** plugin.json is missing, invalid JSON, lacks a string version or names another version
- **THEN** skill checks and the release checker fail, identifying the manifest or version
- **AND** the release command does not push main or a new version tag from that attempt

#### Scenario: Main contains documentation after a release

- **WHEN** Claude fetches main for an installation or update
- **THEN** the plugin may include documentation changes after the release tag with the same numerical version
- **AND** the documentation describes Claude-managed updates without promising every main commit triggers an update
- **AND** missing SOURCE.json leaves skill_source null and revision_match unknown, never a claimed tag match
- **AND** the release archive continues to identify and reproduce its immutable tag commit

#### Scenario: Keep or switch an archive installation

- **WHEN** a consumer already has a manually installed skill
- **THEN** the documentation offers keeping the archive or comparing and preserving local edits before switching
- **AND** switching guidance removes the old manual copy only after that comparison to avoid two active copies
- **AND** plugin updates use /plugin while archive updates retain staging and comparison
- **AND** plugin cache edits are not promised to survive an update

### Requirement: Repository checks preserve the plugin's consumer-only scope

Existing skill tests SHALL check the manifests' fields, types, names, root source and version equality, and SHALL
reject additional plugin components. Manifest files and the skills tree SHALL NOT contain symlinks. Each immediate
skills entry SHALL be a directory with a regular SKILL.md. The repository gate SHALL NOT require Claude Code.

#### Scenario: An unintended component or alternate source

- **WHEN** a manifest gains a component override, another plugin, a non-root source or a mismatched name
- **THEN** structural checks fail with the offending field
- **WHEN** a root commands, agents, hooks, bin, output-styles, workflows, themes, monitors, .mcp.json or .lsp.json
  path, or a symlink in the manifests or skills tree, is added
- **THEN** structural checks fail with the offending path

#### Scenario: Validation without a Claude installation

- **WHEN** a contributor runs the repository gate without the Claude CLI
- **THEN** manifest and scope checks run with the existing Rust/Python tools
- **AND** no CLI download, account access or plugin installation is required

### Requirement: Review checks the vendor format

Design and code review SHALL include real Claude validation evidence, recording CLI version, commands and results.
Validation SHALL remain separate from plugin installation and SHALL NOT require changing account settings.

#### Scenario: Review checks both manifests and the assembled directory

- **WHEN** the design is prepared for tasks or its implementation is prepared for code review
- **THEN** review evidence includes Claude validation of both manifest files and the assembled plugin directory
- **AND** warnings and errors are recorded rather than hidden by a marketplace-only validation
- **AND** successful manifest validation is not described as proof of authentication or end-to-end skill loading

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
