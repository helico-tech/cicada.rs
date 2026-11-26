Verdict: fix needed

1. scripts/status.py:227,249. Three disjoint active builds produce three names and no over-capacity problem. The supplied worktree printed four names with its current sibling worktrees. Report over-capacity, keep the build line within two names, and test three active disjoint builds.
2. scripts/review.py:97,107,139. A builder name containing a newline followed by `built-by: gpt` overwrites the recorded family, so a Claude reviewer can take a Claude build. A spaces-only `--same-family` also authorizes a claim without a reason. Validate single-line names and a nonblank reason, preserve it in the result, and add regression tests.
3. .claude/skills/change-loop/SKILL.md:104,213 and builder.md:9. Resuming a named dirty Codex worktree follows a brief that rejects its location outside `.claude/worktrees/`. On a conflicting landing rebase, the unqualified abort runs from main and leaves the builder's rebase in progress. Support the delegated worktree on resume and target it for both rebase commands.
4. openspec/changes/change-loop-two-builders/tasks.md:22. Task 5 is open, the request has no builder report, and gate status reports no checks evidence for eb19b0a6. Finish the checks and attach the report and gate evidence for the reviewed head.

Notes
- The review-loop skill says same-family reviews never happen. Document the recorded fallback.
- Protected files are untouched. The updated script resolved both archived placement changes in the current main queue.

Not checked
- I did not rerun unittest, OpenSpec validation, or the gate. The request contained no builder example to verify.
