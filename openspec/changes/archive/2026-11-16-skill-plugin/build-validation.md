# Build validation evidence

Date: 2026-11-16. CLI: 2.1.325 (Claude Code). Run by the builder in its worktree on the implemented tree (the
repository root is the plugin), with the commands of task 1. No plugin was installed and no account setting changed.

## Results

| Command | Exit | Verdict | Problems and warnings |
| --- | --- | --- | --- |
| `claude plugin validate .claude-plugin/marketplace.json` | 0 | Validation passed | none |
| `claude plugin validate .claude-plugin/plugin.json` | 0 | Validation passed | directory profile only: LICENSE_MISSING (refused for the public directory), ICON_MISSING (warning); advice: README has no marketplace install line |
| `claude plugin validate .` | 0 | Validation passed | the same as for plugin.json |

The directory-profile findings and the README advice are the same as in `design-validation.md`. They concern a
submission to the public plugin directory, which is out of scope (a private marketplace), and do not change the
verdict. The README advice names a one-step form, `/plugin install cicada --marketplace <owner>/<repo>` on Claude
Code 2.1.275 or later; the docs handoff mentions it as a caveat, not as the documented way.

## Runs

- Task 1, workspace version 0.19.0 in plugin.json: the results above.
- Task 4, the final versioned tree (0.19.1 in Cargo.toml, SKILL.md and plugin.json): the same three exit statuses,
  verdicts, problems, warnings and advice as above.

## Limits

These checks validate the manifests and the layout of the tree only. They do not prove private Git authentication,
installation, update behaviour or interactive skill loading.
