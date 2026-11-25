---
id: container-info.code.1
kind: code
change: container-info
round: 1
base: 1e209b096e387643c0f3690bab50a4c9bbad53dd
head: dc6f2ed471c4a9124a3fb080bb3e46d4dc76fb5f
requested: 2026-11-25T09:17:55+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-info.code.1 <file>`.

## Note from the requester

Version: 0.21.1

Builder report: tasks 1-5 done on feat/container-info (head dc6f2ed4). cargo test --workspace with PostgreSQL 3424 passed;
cicada-containers all features 140 passed; clippy -D warnings, fmt --check, openspec validate --strict clean; docs tests 11,
skill tests 18 passed; gate run --e2e passed (second run; the first failed once on the unrelated
a_node_warns_at_start_when_its_pool_is_smaller_than_its_concurrency_plus_4, which then passed 6 times). No departures from
the design. Docs: handed off, nothing blocks.
