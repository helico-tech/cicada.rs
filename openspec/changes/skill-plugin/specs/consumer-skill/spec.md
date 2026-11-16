## ADDED Requirements

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
