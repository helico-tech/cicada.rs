---
id: inspector-live.code.1
kind: code
change: inspector-live
round: 1
base: 7818622f21ef549122f6e01ab2b2331fe0aae0ee
head: d15e4bc4eb9bfe4188000bbfdee939714e88bbc4
built-by: claude-builder
requested: 2026-11-27T08:30:31+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done inspector-live.code.1 <file>`.

## Note from the requester

Built by Claude (claude-opus-5-5) in /Users/avanwieringen/.t3/worktrees/cicada.rs/t3-77133119-085a161bd810b2ea, branch
feat/inspector-live, head d15e4bc4. Checks: inspector 143 tests, workspace with PostgreSQL 4,028, clippy -D warnings,
fmt, docs tests and doctests, openspec validate --strict. Not run: gate, E2E, AppKit rendering, a real node. The builder
went beyond the design in three places, to confirm: the pill's extra states Connecting (stream open, no event yet) and
Off (no context); the Actors outline's viewport kept as the view last reported it (the live model runs off the main
thread); ⌘R re-arms the 30 s timer when pressed. Manual list: run by code (not seen) or not done.
