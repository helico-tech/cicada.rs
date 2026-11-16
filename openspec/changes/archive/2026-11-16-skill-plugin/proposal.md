# Proposal

## Why

A consumer installs the consumer skill by hand: download the release archive with `gh`, unpack it, copy the folder
into `.claude/skills/`, and repeat that to update (`docs/how-to/install.md`). The owner wants a second, easier way
for Claude Code users (2026-11-16), next to the archive, which stays for every agent. A second skill,
`actor-design`, is coming, so the easier way should carry every consumer skill at once.

Claude Code can install skills as a plugin from a marketplace that a Git repository hosts. A consumer with read access
to this private repository then runs:

```sh
claude plugin marketplace add helico-tech/cicada.rs
claude plugin install cicada@cicada
```

It authenticates with the consumer's own Git credentials (SSH key, or `gh auth setup-git`), and updates through
`/plugin`. Checked against the Claude Code plugin docs on 2026-11-16.

## What Changes

- **This repository is a plugin marketplace.** `.claude-plugin/marketplace.json` lists one plugin, `cicada`, whose
  source is the repository root, and `.claude-plugin/plugin.json` gives its name and version. Claude Code finds the
  top-level `skills/` folder by itself, so every consumer skill is in the plugin with no copy. The skills are named
  `/cicada:cicada` and, later, `/cicada:actor-design`.
- **The plugin's version is the workspace version.** Claude Code updates a plugin only when its version changes, so
  consumers get the skills of a release, not every commit on `main`. The skill test and the release check fail when
  `plugin.json` differs from the workspace version, as they do for `metadata.cicada-version`.
- **The install how-to** gets the plugin way first and keeps the archive way for other agents (docs handoff).
- **An ADR extending ADR 0041**: a second way to ship the consumer skills, and what each way promises.

Decided:

- **Between releases (owner, 2026-11-16): the marketplace follows `main`.** A version bump makes Claude Code take
  `main` as it is at that moment, which can hold documentation-only commits to `skills/` made after the tag. That is
  accepted: the skill routes to the consumer's resolved Cicada source anyway. No pin to a tag, no release branch.
- **The repository root is the plugin (owner, 2026-11-16).** The cache holds a copy of the whole repository (about
  10 MB). A `commands/`, `agents/`, `hooks/` or `bin/` folder added at the top later would load as part of the
  plugin; the design says how the checks catch that.

- **Portable helper path (A1, owner, 2026-11-16).** Keep `<this skill's folder>` and tell the agent to replace it
  with the actual, quoted path. No Claude-only substitution or second command form.
- **Repository checks (B1, owner, 2026-11-16).** Test the manifests, versions and plugin component boundary in the
  existing gate; do not require Claude Code there. Run real Claude manifest validation during design review and
  against the built plugin during code review, recording the CLI version and result.

## Out of scope

A public marketplace or a listing elsewhere. Other agents' plugin systems. Changing the skills' actor guidance or
source-discovery behavior; only helper-path and installation/update wording changes.

## Order

After `node-details`. Before `actor-design-skill`.

## Impact

`.claude-plugin/` (two JSON files), `examples/tests/skill.rs`, `scripts/release-check.sh` or `scripts/gate.py` for
the version check, the release procedure, an ADR, and a docs handoff for `docs/how-to/install.md`. No crate,
guarantee, wire format or schema changes.
