---
id: placement-resource-key.code.2
kind: code
change: placement-resource-key
round: 2
base: a6f6aa90a08486a3bd21f54e64c2bd6d506f5d6a
head: 1d42077be950aa17ccafddb56002b3f26e4d7770
previous: ac78837991efd2ed5e851f84ad1341dbcebbfe3e
requested: 2026-11-26T20:49:46+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-resource-key.code.2 <file>`.

## Note from the requester

Round 2. Review only commit 1d42077b on top of ac788379: the fix of round 1's blocking finding (a read-only lock file
is now opened read-only and locked; no lock only when the file cannot be opened at all), its test, the changelog and
handoff text, and line rewraps (comments and Markdown over 120 columns, the regenerated contract reference). Check the
fix is right on Unix and does not break other platforms; check the rewraps changed no words. Round 1's notes were not
blocking and stay as they are. Short report.
