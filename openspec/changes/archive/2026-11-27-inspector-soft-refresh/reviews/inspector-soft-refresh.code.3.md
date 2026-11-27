---
id: inspector-soft-refresh.code.3
kind: code
change: inspector-soft-refresh
round: 3
base: a2460af94e8ffca8281adab4710607484ebc7128
head: 7d1b32df5bac11059c766cdaca55d5f4450ad0c6
built-by: gpt-builder
previous: 5e19267eead93cabc9cd4a3dea9180bbdf75fc49
requested: 2026-11-27T07:14:53+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-soft-refresh.code.3 <file>`.

## Note from the requester

Built by GPT; code review fixes by the change loop (Claude). Review 2's fix (7d1b32df5bac11059c766cdaca55d5f4450ad0c6): the Order::Id seam is the old index
after the last old row of the longest run of old rows whose fresh positions keep their old order (patience sorting),
so one moved row (Deliver Now) cannot set it; 0 when the range holds no old row. New test: Deliver Now on m440 of 450.
cicada-inspector tests 96 passed, clippy and fmt clean.
