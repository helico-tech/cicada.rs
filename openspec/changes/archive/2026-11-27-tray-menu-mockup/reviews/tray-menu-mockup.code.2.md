---
id: tray-menu-mockup.code.2
kind: code
change: tray-menu-mockup
round: 2
base: 15f64d50e854b8187e085400d889e545a49b6311
head: ff6a4dac0a629b3d44e6b68682faf94bd44b457c
built-by: claude-builder
previous: e666b242210fcf850e6ac9c8b42f0e5d284468b5
requested: 2026-11-27T13:25:35+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done tray-menu-mockup.code.2 <file>`.

## Note from the requester

Built by Claude; round 1's eight fixes by the same builder, head ff6a4dac. The grace phase uses the loop's fallback:
the bar runs from Disconnect to the tray's deadline (grace plus cancel timeout), decision 5 and the spec's
Disconnecting scenario amended accordingly by the builder. Pending the owner's answer, not yet in the design: the two
custom help rows (decision 1) and departure 9 (DEP-CANCELLED grey cross, DEP-SWITCH native switch size); review them
as pending, not as findings. Workspace with PostgreSQL 4,093, tray 64+17, clippy -D warnings, fmt, docs, openspec.
