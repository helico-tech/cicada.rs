---
id: build-loop-docs-handoff.code.2
kind: code
change: build-loop-docs-handoff
round: 2
base: 0a840e96bd4cd78a2845575ea29a4e5b3590bc07
head: a02162b30e66403224584002b0af628b3ec67f32
previous: f3de10bc3c6205ebf580b3a744d60b145970f358
requested: 2026-11-14T19:56:52+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done build-loop-docs-handoff.code.2 <file>`.

## Note from the requester

Version: 0.18.6

Round 2: fixes only the round 1 blocker (commit a02162b, range f3de10b..a02162b). Round 1's report is .reviews/build-loop-docs-handoff.code.1.result.md.

Builder's report: Blocked on docs step 3: after every fix round or rebase, the loop checks each integrated path still equals the docs branch, then the builder runs `cargo test -p cicada-examples --doc`, `--test docs` and `--test skill` in its worktree on the combined result, even when nothing changed; a fresh preparation by the docs session is needed only when a path or behaviour changed; a failing check sends the build back to step 1 of that section. Land step 1: on a branch with integrated owner fixes, the loop checks the paths after any rebase and the builder runs those tests on the rebased branch; the loop merges only when they pass. builder.md states the same rule from the builder's side. docs-handoff.md's Status records the clean-rebase walkthrough with the unconditional checks. The design loop, review briefs and openspec/config.yaml did not repeat the conditional wording and are unchanged; ADR 0045 already says to recheck identity and tests on the combined result.

Checks: docs test 11/11, skill test 8/8, hook unit tests OK, openspec validate --strict valid. The full workspace was not re-run (only briefs and handoff changed; the run before the fix had 2905 passed, 0 failed).
