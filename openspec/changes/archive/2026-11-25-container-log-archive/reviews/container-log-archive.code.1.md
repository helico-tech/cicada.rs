---
id: container-log-archive.code.1
kind: code
change: container-log-archive
round: 1
base: 6132fa3539b47b0d4df3360cfebbe9da0f652636
head: 987dc766bd27742f9db09eaf4d6ffa7dad2f9135
requested: 2026-11-25T19:57:35+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-log-archive.code.1 <file>`.

## Note from the requester

Version: 0.22.1 (the unreleased batch)

Builder report (head 987dc766): tasks 1-5 done. Label cicada.container.prefix added after the job's spec; LogArchiver,
ArchiveSettings, run(stop); the error rules; 21 new archiver tests. Checks: cicada-containers all features 197+46+14,
workspace with PostgreSQL 3561, fmt, clippy -D warnings, openspec --strict, cargo check alone and with docker, examples
docs 11 / doctests 41; gate full pass of 987dc766. Departures: a public const PREFIX_LABEL; LogArchiver is Clone and
ArchiveSettings has Default; tokio no longer optional in cicada-containers; a failed list backs off from the fifth
failure in a row (as the spec says); rounds take containers in turn when there are more than max_parallel.
Docs: handed off, nothing blocks.
