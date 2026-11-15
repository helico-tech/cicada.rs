---
id: conformance-fixtures.code.1
kind: code
change: conformance-fixtures
round: 1
base: e616e729b7528939131adbf559fe8ae732def42d
head: ab0e6e07f16b4d87f371c64abf56c811aa07e78a
requested: 2026-11-15T17:18:33+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done conformance-fixtures.code.1 <file>`.

## Note from the requester

Round 1 of conformance-fixtures, for 0.18.12. Head ab0e6e0 on feat/conformance-fixtures, based on main e616e72 (v0.18.11).

Gate: full checks and E2E passed on 8591bc0, and ab0e6e0 has a docs pass over that full pass (`python3 scripts/gate.py status ab0e6e0`).

Builder notes worth checking:
- `scripts/gate.py release` changed: it now verifies both archives (the skill and the new fixtures) before the tag push, keeps a map of their digests, resumes a draft that already holds one of the two archives, and refuses the old single-digest state. The 0.18.12 release will be the first to run this code. Please check carefully that a resumed or partial release cannot publish a wrong archive or move a tag, and that the old state from the v0.18.9–v0.18.11 releases does not break a new release.
- `scripts/package-skill.py` was refactored onto the shared `scripts/release_archive.py`. A test pins that the skill archive's bytes are unchanged.
- 8591bc0 fixes Python writing bytecode into `scripts/` when it loads the shared helper (`python3 -I` ignores PYTHONDONTWRITEBYTECODE). Commits before it fail the gate on their own.
- Builder's own choices: `status.json` also lists `register` and `list-types` as operations, and git mode 100755 counts as a regular file.
- The number vectors (zmij formatting through serde_json 1.0.151) are pinned in `crates/cicada/tests/fixtures.rs`, per the proposal.
