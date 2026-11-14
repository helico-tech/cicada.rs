---
id: listener-probe-on-close.code.1
kind: code
change: listener-probe-on-close
round: 1
base: 6cb03b13134653b50299ea7aa3a1b89bfa91aedf
head: df6ec371adae1810c5a10f66d407a3e5bc94dec3
requested: 2026-11-14T09:10:21+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done listener-probe-on-close.code.1 <file>`.

## Note from the requester

Version: 0.17.1

Builder's report:

Tasks (all done):
1. Close test strengthened in crates/cicada/src/tests/postgres/changes.rs: a separate observer connection waits up to 15 s for a probe notification and saves the probe's backend PID; before close the test saves the single listener PID and checks it differs from the probe's; after close it waits up to 10 s until neither PID is in pg_stat_activity. Stream, unavailable and no-restart checks stay. On the unfixed code it failed after 10 s with both PIDs still present (recorded in tasks.md).
2. crates/cicada/src/store/postgres/listener.rs: Run::run now runs the probe inside the listener's task with tokio::select! next to receive; probe(&pool, &channel) never returns; the separate probe task is gone. Doc comments updated, version 0.17.1, changelog Fixed entry.
3. docs/openapi.json regenerated for the new version.

Checks: cargo test --workspace with PostgreSQL 2740 passed, 0 failed, no PostgreSQL test skipped; fmt --check and clippy -D warnings pass; cargo check -p cicada with no feature and with postgres pass; openspec validate --strict and the docs test pass after rebasing onto main's proposal-only commits. Not run: Docker E2E.

Departures: none. Finding: the old code leaked the listener's connection too, not only the probe's: both share one pool and the detached probe kept the pool open.

Example:
  let error = tokio::select! {
      error = self.receive(&mut listener) => error,
      never = probe(&pool, &self.channel) => match never {},
  };

Breaking: none.
