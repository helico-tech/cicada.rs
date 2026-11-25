---
id: container-progress.code.1
kind: code
change: container-progress
round: 1
base: b3209f47c8955696f9cb9c0bc27241d5abd09709
head: fe87492dfb0ccaff470649f7230e72f6f1fc926c
requested: 2026-11-25T21:03:14+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-progress.code.1 <file>`.

## Note from the requester

Version: 0.23.0 (BREAKING)

Builder report (head fe87492d): tasks 1-5 done. progressType on Start and EndTarget, start_with_progress,
ContainerProgress, PROGRESS_FILE, progress_seq/progress_hash, contract snapshot rewritten; path check at create and
CICADA_PROGRESS_FILE set after the job's spec; reading and sending at each running poll and at the end; 0.23.0
everywhere with a new unreleased section; OpenAPI regenerated. Checks: cicada-containers 198+59+15, workspace with
PostgreSQL 3576, fmt, clippy -D warnings, openspec --strict, examples 41/11/18; gate full pass. Departures: the state
decoder refuses a hash without a seq and the reverse; only a regular file at exactly the path counts; a job's own
CICADA_PROGRESS_FILE in secret_env is removed; a refused progress send writes a tracing warning.
Docs: handed off, nothing blocks.
