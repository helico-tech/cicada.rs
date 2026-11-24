---
id: pool-size-check.code.1
kind: code
change: pool-size-check
round: 1
base: c03a618fcc8bd854c4fe74f03292001e0d3cff36
head: 253f1b93e69df203fbf390766deee77e3349a3dd
requested: 2026-11-24T21:12:27+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done pool-size-check.code.1 <file>`.

## Note from the requester

Version: 0.21.0 (the unreleased batch)

Builder report (head 253f1b93): tasks 1-5 done. Backend::pool_size() (None by default; PostgreSQL max_pool_size; Store passes through); private pool_warning (pool < concurrency + 4, no overflow) logged by start with tracing::warn! before spawning, only when the node hosts a type; a PostgreSQL start test with a thread-local subscriber (fails without the warning). fmt, clippy clean; cargo test --workspace with PostgreSQL 3418; tests::postgres 36; docs 11; skill 18; openspec strict valid; gate.py full pass of 253f1b93.
Departures: the warning also names the concurrency settings (NodeConfig::concurrency, --concurrency / CICADA_CONCURRENCY), as the spec says "the pool and concurrency settings"; existing PostgreSQL Cluster tests (pool 6, concurrency 4) now log the warning and still pass.
Docs: handed off, nothing blocks.
