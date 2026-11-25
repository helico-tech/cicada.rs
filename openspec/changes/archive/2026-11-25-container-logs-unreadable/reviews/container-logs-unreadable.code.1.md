---
id: container-logs-unreadable.code.1
kind: code
change: container-logs-unreadable
round: 1
base: a8d529308f96b963fd5d6a3b6d1faefa0e1a955c
head: cb7558e777013143504be9cc998d73531db8c0b0
requested: 2026-11-25T16:44:36+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-logs-unreadable.code.1 <file>`.

## Note from the requester

Version: 0.22.0 (the unreleased batch)

Builder report: tasks 1-4 done; rebased onto main a8d52930 (head cb7558e7). Docker's 501 on a log read gives Unsupported
(map::log_read_failed, used by logs and logs_since); rustdoc of the trait's log calls, ErrorKind::Unsupported and
DockerRuntime updated; the actor records (no logs: …) at once for Unsupported without counting or retrying; two new
actor tests fail without the fix; a new E2E step with --log-driver none. Pre-rebase: workspace with PostgreSQL 3510,
E2E class 31, gate full and e2e pass of 6d1c54a7. Post-rebase: cicada-containers 170+45+9, docs 11, fmt, clippy pass;
the landing gate runs on main after merge. Departure: the end's logs read "(no logs: unsupported: logs of <name>: …)",
the kind included, as today's "(no logs: {e})" format. Docs: handed off, nothing blocks.
