# Design

## Scope and decisions

One behavior: install the consumer skills as a Claude Code plugin, alongside the release archive. This fits four
implementation tasks plus finishing checks: manifests, checks/release guard, skill wording, and ADR/docs handoff.
No runtime, schema, wire format, guarantee or public Rust API changes. ADR 0049 extends ADR 0041's distribution rule.

1. **Root plugin, following main** (owner, 2026-11-16). Add `.claude-plugin/marketplace.json` with exactly `name`,
   `owner`, `description`, `plugins`: name `cicada`, owner `{ "name": "helico-tech" }`, a nonempty description, and one entry
   `{ "name": "cicada", "source": "./" }`. The repository's default branch is main. No tag/ref pin, remote plugin
   source, marketplace version or component overrides. A subfolder plugin would require copying or moving skills.
2. **One plugin manifest.** `.claude-plugin/plugin.json` has exactly `name`, `version`, `description`, `author`,
   `repository`: name `cicada`, workspace version as a string, a nonempty description of the consumer skills,
   author `{ "name": "helico-tech" }`, repository `https://github.com/helico-tech/cicada.rs`.
   Default `skills/` discovery includes every consumer skill without listing or copying it. Contributor loops under
   `.claude/skills/` are not components. The cache may contain the whole repository; that does not make its docs the
   consumer's authority. No new install hooks, servers, commands, dependencies or credentials are bundled.
3. **Version and provenance.** Release preparation bumps plugin.json with workspace and skill metadata. The skill
   test checks equality; `scripts/release-check.sh` uses its existing jq prerequisite to require a parseable manifest
   with a string version equal to the release version. Missing, malformed or mismatched manifests fail before push.
   Update the release tests' fixture manifests and cover those failures. The marketplace does not repeat the version.
   Plugin updates use Claude's version/cache rules and the main tree it fetches; fresh installation or reinstallation
   can also get later documentation commits with the same version. This is not a tag-exact distribution promise.
   No generated SOURCE.json for the plugin: existing `skill_source: null` and `revision_match: unknown` remain honest.
   The archive, its layout, tag-based packager and SOURCE.json remain unchanged.
4. **Portable folder reference, A1** (owner, 2026-11-16). Keep `<this skill's folder>` and explain replacement with
   the actual path quoted for the shell, running from the consumer project. Do not add `${CLAUDE_SKILL_DIR}` or a
   second command form. Both distributions use the same SKILL.md and resolver. Routing still uses Cargo's resolved
   source, not the plugin checkout; updating either distribution never changes the consumer's dependency.
5. **Repository checks, B1** (owner, 2026-11-16). Extend `examples/tests/skill.rs` with checks of the exact manifest
   fields/shapes above, one root plugin, matching versions and at least the existing `skills/cicada/SKILL.md`.
   Each immediate skills/ entry must be a real directory with a regular SKILL.md; permit future skills without a
   hardcoded count. Reject symlinks within skills/ and at the manifest paths, so copied content stays self-contained.
   Reject root `commands/`, `agents/`, `hooks/`, `bin/`, `output-styles/`, `workflows/`, `themes/`, `monitors/`,
   `.mcp.json` and `.lsp.json`, including symlinks. Exact manifest keys reject custom component paths/inline config.
   These are checks of this repository's chosen shape, not a reimplementation of Claude's schema or a sandbox.
   Temporary fixtures prove rejection of bad JSON/types, a second entry, a changed source, mismatched names/version,
   a component override, each forbidden root path and symlinks; a second ordinary skill is accepted. Existing gate
   full/workspace and docs/skill runs already cover these tests; no gate policy or Claude CLI dependency is added.
6. **Vendor validation in review.** Validate candidate manifests in a temporary directory with the current skills
   before ready; record CLI version, commands, errors and warnings in review evidence. Validate both explicit JSON
   files and the assembled directory so marketplace detection cannot hide the plugin check. The builder repeats
   validation on the implemented tree for code review. No plugin installation or account-setting change is needed.
   A passing validator is manifest/layout evidence, not proof of private Git authentication or end-to-end loading.
7. **Updates and migration.** Amend SKILL.md's section 7 to distinguish a plugin managed through `/plugin` from a
   staged archive compared before replacement. Plugin cache edits are not durable customizations. Existing archive
   users may keep that installation; switching requires comparing/preserving their edits before removing the old
   manually installed skill, to avoid two copies. Do not silently migrate, uninstall or configure automatic updates.

## Documentation and verification

The builder writes `docs-handoff.md`; the docs session owns the prose. Hand off `docs/how-to/install.md` sections
`Install the coding-agent skill` and `Update the installed skill`: plugin-first install commands from the proposal,
private-repository Git access, `/cicada:cicada`, plugin updates, archive alternative, switching/customization guidance,
main-versus-tag provenance, and unchanged resolved-source routing. Preserve the existing section anchors.
Hand off `docs/how-to/releasing.md`'s version preparation and post-release version reset to include plugin.json,
plus review-only Claude validation. `docs/agents.md` already links to installation and needs no text change.
No Rust doc block, relative link, skill route or G/S guarantee row needs changing; no new guide/example is introduced.
Unexpected protected-prose failures block landing for a docs-owner fix. Builder owns skill text, ADR/index and tests.
Verify targeted skill and release tests, existing archive/resolver tests, docs links/routes and doctests. Finishing
uses the full committed-tree gate including PostgreSQL and Docker, plus `openspec validate skill-plugin --strict`.

## Sources

Checked 2026-11-16: [manifest](https://code.claude.com/docs/en/plugins-reference),
[marketplace](https://code.claude.com/docs/en/plugins/marketplace-reference),
[validation](https://code.claude.com/docs/en/plugins/cli-reference). These define discovery and validation; our
checks deliberately enforce the narrower shape above. The supported component list may need updating with Claude.
