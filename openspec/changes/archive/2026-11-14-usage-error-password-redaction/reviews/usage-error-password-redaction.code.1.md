---
id: usage-error-password-redaction.code.1
kind: code
change: usage-error-password-redaction
round: 1
base: 47aa322a5b7614680fcfa974a0838c8671f94157
head: 594060dfc83ff5e6d5abfab67e5e28df5726a84f
requested: 2026-11-14T11:33:22+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done usage-error-password-redaction.code.1 <file>`.

## Note from the requester

Version: 0.17.3

Owner rule for this build (decision 24a, until build-loop-docs-handoff lands): the docs session owns README and hand-written docs; the builder leaves them alone and writes user-facing prose into openspec/changes/<change>/docs-handoff.md. Review the handoff instead of guide edits.

Builder's report:

Tasks (all done):
1. hide_passwords (crates/cicada-app/src/command.rs) no longer stops at `'`. When the scan reaches `scheme:` followed by `//`, it ends the current address before that scheme so the next scan handles it. Every helper case is also checked twice (a second pass changes nothing). Doc comments updated there and on AppError::Usage in lib.rs. The new App test fails on the old code.
2. New test in crates/cicada-app/tests/app.rs for three cases: a rejected argument with S3'CRET, CICADA_LISTEN with an apostrophe in user and password, and --types 'a://h,,b://u:secret@k'. Checks error kind, exit code 2, the start of the message, no secret fragment left, nothing started.
3. Changelog entry and version. The app-guide prose is in openspec/changes/usage-error-password-redaction/docs-handoff.md with agreed text; README and guides untouched. The generated CLI reference is still current.
4. Finishing checks.

Checks: cargo test --workspace with PostgreSQL 2712 passed, 0 failed; fmt --check and clippy -D warnings clean; openspec validate --strict passes. Not run: E2E.

Departures: none. The version bump also changed docs/openapi.json (regenerated) and `cicada-version` in skills/cicada/SKILL.md; tests and release-check.sh require both to match.

Example (helper test):
  ("unexpected argument 'postgres://u:S3'CRET@h/db' found", "unexpected argument 'postgres://u:***@h/db' found"),
  ("'a://u:p'q@h','b://v:r's@k'", "'a://u:***@h','b://v:***@k'"),
  ("a://h,,b://u:secret@k", "a://h,,b://u:***@k"),

Breaking: none.
