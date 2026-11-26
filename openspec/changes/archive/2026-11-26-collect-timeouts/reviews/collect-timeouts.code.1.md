---
id: collect-timeouts.code.1
kind: code
change: collect-timeouts
round: 1
base: 23d2a2a45954bb4552ffafb15deb43f9f28bd450
head: 30b76d4bedd550f1f5952b6527ca86ab2aaac21f
requested: 2026-11-26T02:10:52+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done collect-timeouts.code.1 <file>`.

## Note from the requester

Version: 0.23.1 (additive)

Builder report (head 30b76d4b): tasks 1-4 done, task 5's checks passed (left unticked to keep the gate record; the
orchestrator ticks it at landing). CopyLimits::max_read (default 4 x max_bytes) and with_max_read, Docker and the fake
stop at the same file; giving up after three failures with the MAX_TURN rustdoc updated; warnings with a test in
tests/warnings.rs. Checks: workspace with PostgreSQL 3628, clippy -D warnings, fmt, openspec --strict, examples docs and
doc tests; gate full pass of 30b76d4b. Departures: the collecting warning also fires for runtime errors (same retry
path); on giving up the counts come from store.files (0 and a warning if it fails); three old tests now set a large
max_read; tracing-subscriber is a new dev-dependency of cicada-containers. Docs: handed off, nothing blocks.
