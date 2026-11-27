---
id: inspector-soft-refresh.code.4
kind: code
change: inspector-soft-refresh
round: 4
base: a2460af94e8ffca8281adab4710607484ebc7128
head: 49a7e10b401d7a7b0bcb4a25f0fdb8fe641f9122
built-by: gpt-builder
previous: 7d1b32df5bac11059c766cdaca55d5f4450ad0c6
requested: 2026-11-27T07:20:28+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-soft-refresh.code.4 <file>`.

## Note from the requester

Built by GPT; code review fixes by the change loop (Claude). Review 3's fix (49a7e10b401d7a7b0bcb4a25f0fdb8fe641f9122) drops the seam heuristic: SoftPaged
gets a Sort (the server's order) beside Key (identity), and every list uses Order::Key-style ranges. Messages sort by
(to as a string, deliver_at, seq, id), list_pending's documented order (store/mod.rs:259; COLLATE "C"); Outcomes by
(finished_at, id) (store/mod.rs:284). Design decision 1 revised in the worktree. Unit tests for the actor-block and
bulk Deliver Now cases in pager.rs. cicada-inspector 98 passed; clippy (inspector, tray, all features) and fmt clean.
