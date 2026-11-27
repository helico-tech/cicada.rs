---
id: inspector-soft-refresh.code.2
kind: code
change: inspector-soft-refresh
round: 2
base: a2460af94e8ffca8281adab4710607484ebc7128
head: 5e19267eead93cabc9cd4a3dea9180bbdf75fc49
built-by: gpt-builder
previous: 8530a2f01bd18be4aac467fa1ded475e3b066941
requested: 2026-11-27T07:09:42+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-soft-refresh.code.2 <file>`.

## Note from the requester

Built by GPT; the code review 1 fix (one commit on top, 5e19267eead93cabc9cd4a3dea9180bbdf75fc49) was made by the change loop (Claude): the Order::Id seam
keeps old rows after the last old row the new range holds (all old rows when it holds none), with a new test that
inserts rows ahead of the seam; one existing test's expectation changed accordingly (a range holding no old row keeps
all old rows, not refreshed). cicada-inspector tests 95 passed, clippy and fmt clean.
