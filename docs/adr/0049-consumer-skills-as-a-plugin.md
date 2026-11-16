# 0049. Consumer skills as an optional Claude Code plugin

Date: 2026-11-16

## Status

Accepted for `skill-plugin` after design review and the owner's approval on 2026-11-16. Extends ADR 0041's
distribution convention; no runtime guarantee changes.

## Context

The release archive works across agents, but Claude Code users can install and update a set of skills through a
Git-backed plugin marketplace. The owner chose the repository root as the plugin and main as its source, accepting
a full repository cache and documentation changes between releases. The archive remains the tag-exact distribution.

## Decision

- Add a marketplace named cicada with one root-source plugin, also cicada. Default skills/ discovery carries the
  consumer skills; contributor loops under .claude/skills are not plugin components. No separate copy of the skills.
- Match plugin.json's version to the workspace and check it in skill tests and before release publication. Keep the
  version out of the marketplace entry. Follow main without a tag pin or release branch; the plugin's numerical
  version does not identify immutable content. Fresh installs can include later documentation with that version.
- Keep the portable installed-folder instruction. Both distributions run the same resolver against the consumer's
  dependency. A plugin without SOURCE.json reports unknown revision identity; do not synthesize archive provenance.
- Enforce the chosen manifest shape and absence of extra default components in repository tests. The gate needs no
  Claude CLI. Record real Claude manifest/layout validation during design and code review, including warnings.
- Explain plugin-managed updates and archive comparison separately. Preserve local edits when switching and avoid
  duplicate installations. Changing the skill installation does not change the consumer's dependency.

## Consequences

Claude Code users gain a single installation for all consumer skills; other agents keep the archive. No public
marketplace, bundled credentials, install hooks or automatic migration is introduced. Repository checks catch known
structural drift, not future vendor discovery changes or authentication failures. The docs session maintains the
installation and release guidance from the builder's handoff. Existing immutable tags and archives are unaffected.
