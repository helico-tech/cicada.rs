---
id: tray-menu-mockup.code.7
kind: code
change: tray-menu-mockup
round: 7
base: 15f64d50e854b8187e085400d889e545a49b6311
head: 56318c72ef661f0b67232802baf77dca5f76b53c
built-by: claude-builder
previous: bfc599fec3b1c30381039a9e34307f4e46842617
requested: 2026-11-27T15:35:23+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done tray-menu-mockup.code.7 <file>`.

## Note from the requester

Builder's round 6 fixes, head 56318c72: the failure row records a kind found by type in the source chain (Connection
refused, No answer, else Connection failed) and shows no error text; Host not found, TLS error and HTTP <status>
follow with the proposed change source-error-kinds (the client stringifies its errors today; loop's pick QA-1).
Recent Turns: the doubtful check runs on the whole error. Decisions 3 and 4 and the handoff updated. Tray 69+17,
clippy, fmt, openspec. Pending the owner, not findings: two custom help rows, DEP-CANCELLED, DEP-SWITCH.
