---
id: tray-menu-mockup.code.3
kind: code
change: tray-menu-mockup
round: 3
base: 15f64d50e854b8187e085400d889e545a49b6311
head: c84f0fe989b3d034670eb75ab767a4ce80e74256
built-by: claude-builder
previous: ff6a4dac0a629b3d44e6b68682faf94bd44b457c
requested: 2026-11-27T13:32:12+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done tray-menu-mockup.code.3 <file>`.

## Note from the requester

Builder's round 2 fixes, head c84f0fe9: URLs in error text replaced by scheme and host via the url crate (<url> when
unparseable), trailing punctuation kept as text; ports from the parser, a default port only when written in digits;
a Renders guard deferring nested renders; text_space budgets derived from the boards' CSS (Main's context row 242 pt).
Tray 66+17 (67+17 inspector), cicada-app 206, clippy, fmt, cargo deny. Workspace not rerun this round. Still pending
the owner: the two custom help rows, DEP-CANCELLED and DEP-SWITCH (not findings).
