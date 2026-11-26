Verdict: fix needed

1. .claude/skills/change-loop/builder.md:18. The brief now accepts the named delegated worktree, then runs a plain rebase. With an unstaged tracked edit, Git refuses even when the branch is already based on main. I reproduced this in a temporary repository. Make the resume path preserve dirty edits and defer the rebase until the builder has a clean tree.

2. scripts/review.py:135,161,205. Design reviews bypass reviewer-name validation. A multiline name can add a same-family field to the taken marker, and done appends it as though a fallback occurred. Validate provided names for both review kinds while keeping design names optional, and add a regression test.

Verified: Over-capacity reporting and the two-name cap, code-review name and reason validation, recording a valid fallback, the fallback documentation, and the worktree-scoped rebase abort. All 34 targeted tests and git diff --check passed.

Unchecked: OpenSpec validation and the full suite. Task 5 and the gate are handled at landing.
