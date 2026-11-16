---
id: skill-plugin.code.1
kind: code
change: skill-plugin
round: 1
base: 9d30233128218abfe33b01b6b981ab2a03b403ad
head: d263b2a81ededfcf556b1666780906b9d1611fd6
requested: 2026-11-16T10:51:52+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done skill-plugin.code.1 <file>`.

## Note from the requester

Version: 0.19.1

Builder report (feat/skill-plugin, head d263b2a):

Tasks
1. Manifests: done. .claude-plugin/marketplace.json and .claude-plugin/plugin.json, the repository root as the plugin.
2. Checks: done. examples/tests/skill.rs checks the manifests and the root's plugin components; scripts/release-check.sh checks the plugin version before a release. Both have tests for the bad cases.
3. Skill wording: done. SKILL.md section 1 says to use the real, quoted install path; section 7 explains plugin updates, archive updates and switching. The helper was run from a plugin-style and an archive-style copy; both reported `self`.
4. Version and handoff: done. 0.19.1 in the workspace, Cargo.lock, the skill, plugin.json and docs/openapi.json. Changelog entry and docs-handoff.md written.
5. Finishing: done.

Checks
- skill tests 10 passed; docs tests 11 passed; doctests 29 passed; release script tests 22 passed.
- openspec validate skill-plugin --strict: valid. cargo fmt --all --check: clean.
- gate.py status: full pass, e2e pass (docs record missing, expected for a builder).
- claude plugin validate (CLI 2.1.325) on both manifests and the repository root: exit 0; only public-directory findings (no LICENSE, no icon), recorded in build-validation.md.
- Not checked: real installation, private Git access, the skill actually loading in Claude Code.

Departures
- Also edited .claude/skills/build-loop/builder.md and .claude/skills/review-loop/code.md so the version steps name plugin.json (not protected files).
- The validator suggests a one-step form `/plugin install cicada --marketplace helico-tech/cicada.rs`; the handoff names it only as a caveat.

Breaking: none. Docs: handed off, no blockers.
