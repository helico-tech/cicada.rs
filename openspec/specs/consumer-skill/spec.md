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
