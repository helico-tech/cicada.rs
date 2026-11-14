---
id: build-loop-docs-handoff.code.1
kind: code
change: build-loop-docs-handoff
round: 1
base: 0a840e96bd4cd78a2845575ea29a4e5b3590bc07
head: f3de10bc3c6205ebf580b3a744d60b145970f358
requested: 2026-11-14T19:54:00+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done build-loop-docs-handoff.code.1 <file>`.

## Note from the requester

Version: 0.18.6 (process-only; no library code)

Owner rule for this build: README and hand-written docs belong to the docs session; protected prose (AGENTS.md included, by the build loop's instruction, although the hook does not cover AGENTS.md) is in openspec/changes/build-loop-docs-handoff/docs-handoff.md. Review the handoff instead of those files.

Builder's report:

Tasks 1–5 done. builder.md: a Documentation section (ownership and its precedence over tasks; a docs-handoff.md for every change with its template; reporting a build blocked on docs and taking in owner-prepared files; the version step also sets the skill's cicada-version and regenerates docs/openapi.json; a seventh report item, the docs state). build-loop SKILL.md: a "Blocked on docs" section; a Docs line in Present; Land rechecks after every rebase, including clean ones, points at docs/how-to/releasing.md and names the archived handoff. Design loop, both review briefs and openspec/config.yaml: name the doc blocks a change breaks, ask for a handoff instead of prose; code review checks the handoff and any owner-prepared files; a failing check still blocks even when the handoff lists it. ADR 0045 covers AGENTS.md and names the briefs; index row says 0.18.6. Version 0.18.6 in Cargo.toml, Cargo.lock, skill metadata and OpenAPI, with a process-only changelog entry.

Checks: fmt --check and clippy clean; cargo test --workspace with PostgreSQL 2905 passed, 0 failed; docs test 11/11, skill test 8/8, hook unit tests 8/8 (hook scripts unchanged); openspec validate build-loop-docs-handoff --strict valid (validate --all --strict shows 9 failures, all in untouched files: older specs' long-requirement warnings and three proposal-only changes). Walkthroughs of the additive, blocking, owner-unavailable, changed-head, clean-rebase and release-pin cases against the hook's actual rules.

Departure: the handoff flags a risk for the docs session: its regular run merges the docs branch into main, so prepared prose for a blocked build would reach main before its code and break doctests; it must hold that merge until the feature lands.

New concepts: docs handoff (a file each change writes: what users must learn, where it goes, the version for install pins); blocked on docs (a change that cannot pass its checks without new prose waits until the docs session prepares the exact files).
Breaking: none.
