# Design validation evidence

Date: 2026-11-16. CLI: 2.1.325 (Claude Code).

A temporary candidate held the two manifests specified by design.md, the unchanged skills/ tree, and copies of
the repository README.md and CLAUDE.md (where present). No plugin was installed and no account setting changed.
The candidate used workspace version 0.19.0. The temporary directory was removed afterwards.

The initial candidate lacked a marketplace description; adding the nonempty description in the final design
removed that warning. The initial snapshot also omitted README.md; the final snapshot includes the real file.

## Final candidate results

### claude plugin validate <candidate>/.claude-plugin/marketplace.json

Exit status: 0.

```text
Validating marketplace manifest: <candidate>/.claude-plugin/marketplace.json

✔ Validation passed
```

### claude plugin validate <candidate>/.claude-plugin/plugin.json

Exit status: 0.

```text
Validating plugin manifest: <candidate>/.claude-plugin/plugin.json

Plugin directory checks for <candidate>. The plugin directory is the catalog that plugins are submitted to; this is a subset of what it runs.

✘ Found 1 problem the plugin directory refuses a submission for:

  ❯ LICENSE: License missing (LICENSE_MISSING). This plugin has no LICENSE file and plugin.json has no `license`. The directory requires one of them.
    Fix: Add a LICENSE file to the plugin folder, or set `license` (an SPDX id) in plugin.json. A LICENSE at the repository root also counts for a plugin in a sub-folder.

⚠ Found 1 plugin directory warning:

  ❯ .claude-plugin/plugin.json icon: No icon (ICON_MISSING). No icon found in the plugin. The validator looked for `icon` in plugin.json and for a file at .claude-plugin/icon.png or .svg, assets/icon.png or .svg, or a similar path.
    Fix: Add a square PNG to the plugin folder at .claude-plugin/icon.png, or set `icon` in plugin.json to the path of a square PNG or JPEG in the plugin: 512 to 2048 px on each side, under 2 MB. SVG and WebP are not accepted. That file may become the listing icon only once: the first time the plugin is saved or submitted in the developer portal. Adding or changing it later does not change the icon.

About these checks:

  ❯ This folder also holds a marketplace manifest. The plugin directory treats such a folder as a plugin, so these checks cover only the plugin in this folder. To check another plugin that the marketplace lists, run validate on that plugin's folder.

These plugin directory checks do not change the result below. To fail on problems the directory refuses a submission for, run this command with --profile directory.

The directory also runs a safety scan and a policy review that this command does not.

ℹ Advice (does not change the verdict):

  ❯ <candidate>/README.md: This README has no line that installs "cicada" from a marketplace. Add the install line, which works in one step on Claude Code 2.1.275 or later. Validate the folder that holds its marketplace.json to have <owner>/<repo> filled in, and the name its marketplace lists it under.
      /plugin install cicada --marketplace <owner>/<repo>

✔ Validation passed
```

### claude plugin validate <candidate>

Exit status: 0.

```text
Validating marketplace manifest: <candidate>/.claude-plugin/marketplace.json

Plugin directory checks for <candidate>. The plugin directory is the catalog that plugins are submitted to; this is a subset of what it runs.

✘ Found 1 problem the plugin directory refuses a submission for:

  ❯ LICENSE: License missing (LICENSE_MISSING). This plugin has no LICENSE file and plugin.json has no `license`. The directory requires one of them.
    Fix: Add a LICENSE file to the plugin folder, or set `license` (an SPDX id) in plugin.json. A LICENSE at the repository root also counts for a plugin in a sub-folder.

⚠ Found 1 plugin directory warning:

  ❯ .claude-plugin/plugin.json icon: No icon (ICON_MISSING). No icon found in the plugin. The validator looked for `icon` in plugin.json and for a file at .claude-plugin/icon.png or .svg, assets/icon.png or .svg, or a similar path.
    Fix: Add a square PNG to the plugin folder at .claude-plugin/icon.png, or set `icon` in plugin.json to the path of a square PNG or JPEG in the plugin: 512 to 2048 px on each side, under 2 MB. SVG and WebP are not accepted. That file may become the listing icon only once: the first time the plugin is saved or submitted in the developer portal. Adding or changing it later does not change the icon.

About these checks:

  ❯ This folder also holds a marketplace manifest. The plugin directory treats such a folder as a plugin, so these checks cover only the plugin in this folder. To check another plugin that the marketplace lists, run validate on that plugin's folder.

These plugin directory checks do not change the result below. To fail on problems the directory refuses a submission for, run this command with --profile directory.

The directory also runs a safety scan and a policy review that this command does not.

ℹ Advice (does not change the verdict):

  ❯ <candidate>/README.md: This README has no line that installs "cicada" from a marketplace. Add the install line, which works in one step on Claude Code 2.1.275 or later. In it, replace <owner>/<repo> with the repository that holds the marketplace.
      /plugin install cicada --marketplace <owner>/<repo>

✔ Validation passed
```

## Limits

These checks validate candidate manifests and copied layout only. They do not prove private Git authentication,
installation, update behavior or interactive skill loading. Public-directory license/icon advice is out of scope:
this is a private marketplace, not a public-directory submission. A root CLAUDE.md is contributor guidance and
is not loaded by the plugin. The builder must repeat validation against the actual implemented tree.

`openspec validate skill-plugin --strict` passed before the design review request.
