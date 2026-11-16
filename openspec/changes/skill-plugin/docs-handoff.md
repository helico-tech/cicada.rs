# Docs handoff for skill-plugin

Version: 0.19.1. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The consumer skills can be installed as one Claude Code plugin, next to the release archive (ADR 0049).

- `.claude-plugin/marketplace.json`: a marketplace named `cicada` with one plugin, `cicada`, whose source is the
  repository root (`./`). `.claude-plugin/plugin.json`: name `cicada`, the workspace version, a description, author
  and repository. Claude Code finds `skills/` by itself, so every consumer skill is in the plugin with no copy; the
  skill `cicada` is `/cicada:cicada`.
- The plugin follows `main`. Claude Code takes a new copy when the plugin's version changes, so a fresh install or an
  update can hold documentation committed after the release tag under the same version. A plugin has no
  `SOURCE.json`: the resolver reports `skill_source: null` and `revision_match: unknown` for it, never a match. The
  release archive, its layout and `SOURCE.json` are unchanged and stay the tag-exact way.
- `skills/cicada/SKILL.md` section 1 says to replace `<this skill's folder>` with the actual installed path, quoted
  for the shell, running from the project; section 7 tells plugin updates (`/plugin`) apart from archive updates
  (staged and compared), and how to switch without losing local edits or keeping two copies.
- Checks: `examples/tests/skill.rs` (`the_plugin_carries_the_consumer_skills_and_nothing_else`,
  `a_changed_manifest_another_component_or_a_link_is_named`) checks the manifests' exact fields, the version, real
  skill folders with a regular `SKILL.md`, no symbolic links and no other plugin component at the root.
  `scripts/release-check.sh` refuses a release whose `plugin.json` is missing, not JSON, or has no string version
  equal to the tag's (`scripts/test_gate_release.py`,
  `test_a_missing_malformed_or_stale_plugin_version_publishes_nothing`).
- No Rust API, wire format, contract, schema or guarantee changes.

## 1. Install the skill as a Claude Code plugin

**Where.** `docs/how-to/install.md`, `Install the coding-agent skill`. Keep the heading and its anchor; put the plugin
way first and the archive way after it, as the way for other agents.
**Text.** For Claude Code, install the plugin from this repository's marketplace. It needs Git read access to the
private repository with the user's own credentials (an SSH key, or `gh auth setup-git` for HTTPS):

```sh
claude plugin marketplace add helico-tech/cicada.rs
claude plugin install cicada@cicada
```

The skill is then `/cicada:cicada`; later consumer skills come with the same plugin. Other agents, or a user who wants
the exact files of a release tag, use the release archive as now. The helper command shown for the archive
(`python3 .claude/skills/cicada/scripts/cicada_source.py`) becomes, in general, the installed skill folder's path,
quoted: the skill tells the agent to do that.
**Caveats.** The plugin follows `main`, not a tag: its version is the workspace version, but a fresh install can hold
documentation committed after that release. It has no `SOURCE.json`, so the helper reports `revision_match: unknown`.
The project's resolved Cicada source still wins in both cases; installing the plugin does not change the project's
dependency. A passing install depends on the user's Git access; the repository does not ship credentials. Claude Code
2.1.275 and later also offer a one-step `/plugin install cicada --marketplace helico-tech/cicada.rs` (seen in the
validator's advice, not tested here); the two commands above are the documented way.

## 2. Update the installed skill

**Where.** `docs/how-to/install.md`, `Update the installed skill`. Keep the heading and its anchor.
**Text.** A plugin is updated by Claude Code through `/plugin`; it takes a new copy when the plugin's version changes,
which happens with a Cicada release, not with every commit on `main`. Edits to the plugin's copy in Claude Code's cache
do not survive an update. An archive installation is updated as now: stage, compare with `diff -r`, then replace.
Switching from an archive to the plugin: compare the installed `.claude/skills/cicada` with the plugin's skill and keep
the local changes you need elsewhere, then remove `.claude/skills/cicada`, so that only one copy is active. Keeping the
archive installation is fine. Nothing switches, uninstalls or turns on automatic updates by itself.
**Caveats.** Updating either kind of installation and updating the Cicada dependency are separate actions.

## 3. Release preparation and the version reset

**Where.** `docs/how-to/releasing.md`, step 1 of the procedure and the paragraph after the prerequisites (`When the next
implementation change lands, …`).
**Text.** Step 1: keep `version` in `.claude-plugin/plugin.json` equal to the workspace version too, beside
`metadata.cicada-version` in `skills/cicada/SKILL.md`; the skill test and the release checker enforce both. The reset
paragraph: set the next version in `Cargo.toml`, the skill metadata and `.claude-plugin/plugin.json`. Add that code
review records `claude plugin validate` of `.claude-plugin/marketplace.json`, `.claude-plugin/plugin.json` and the
repository root (CLI version, commands, warnings); the gate does not need the Claude CLI.
**Caveats.** The validator's public-directory findings (no LICENSE, no icon) concern a submission to the public plugin
directory and do not apply to this private marketplace. A passing validation is not proof of Git access or loading.

`docs/agents.md` already links to the installation guide and needs no change.

## Blocking

None. No Rust doc block, relative link, skill route or guarantee row needed new prose.

## Status

All prose above is proposed text; none is written. Checks run on the final tree: `cargo test -p cicada-examples --test
skill` (10 passed), `--test docs`, `--doc`, `python3 -m unittest discover -s scripts -p 'test_gate_release.py'` (22
passed) and the gate; results in the build report. `claude plugin validate` evidence is in `build-validation.md`.
