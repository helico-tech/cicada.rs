---
id: app-tasks.code.1
kind: code
change: app-tasks
round: 1
base: a3c9545df6d7c9aa3fd6f57c612902b961de6335
head: 75fabb3766167e32cd6e0a98902bec6b74cba368
requested: 2026-11-25T17:59:47+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done app-tasks.code.1 <file>`.

## Note from the requester

Version: 0.22.0 (the unreleased batch)

Builder report (head 75fabb37): tasks 1-4 done. App::task and Stopping; duplicate names exit 2; an HTTP worker refuses a
task (exit 2) and the worker help says so; docs/reference/cli.md regenerated; the life of a task in node.rs; 7 tests in
crates/cicada-app/tests/tasks.rs plus the HTTP refusal in worker_http.rs. Checks: fmt, clippy -D warnings, workspace with
PostgreSQL 3523, examples docs 11 / doc 37 / skill 18, openspec --strict; gate full and e2e pass of 75fabb37.
Departures: serve keeps HTTP answering (/ready 503) until both the node and the tasks stopped, then the 5 s drain (the
worst case stays grace + cancel timeout + drain); each task is polled once before `Listening on` is written; a failed
read of the listen address also stops the tasks and closes the store. Docs: handed off, nothing blocks.
