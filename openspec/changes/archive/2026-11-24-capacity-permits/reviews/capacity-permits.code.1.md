---
id: capacity-permits.code.1
kind: code
change: capacity-permits
round: 1
base: 0db7f09ca502a2b50314a6e878f83543e7c21de9
head: 9bd00e2dbff183e869494c67dfe21963679fdae6
requested: 2026-11-24T13:30:53+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done capacity-permits.code.1 <file>`.

## Note from the requester

Version: 0.20.8

Builder report (head 9bd00e2): tasks 1-6 done. cargo test --workspace with PostgreSQL 3237 passed (at 532ee77); cicada-actors 575 passed incl. --test permits 27 and wire_formats 25; docs 11, skill 18; fmt, clippy -D warnings, openspec strict clean; gate.py full pass of 9bd00e2. E2E at release.
Choices: the decoder refuses grant number 0; nextGrant at u64::MAX is refused for good. After the first build, the change loop closed an open question: Audit sends no check to a holder whose grant notice has not committed (notified false), and a check for a waiter granted before the check runs sends nothing, so a grant number reaches a holder only through `granted` (design decision 10 and the spec updated; two tests). The task text says 0.20.7; 0.20.7 was released first, so 0.20.8 as instructed.
Docs: handed off, nothing blocks.
