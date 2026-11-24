---
id: container-copy-out.code.1
kind: code
change: container-copy-out
round: 1
base: 0f6524145e8c66f10e29f0849d2a9ba3ff2c960a
head: 8e25e107c05b28b5c38a4afd4b4ac95246b96878
requested: 2026-11-24T18:31:33+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-copy-out.code.1 <file>`.

## Note from the requester

Version: 0.20.9

Builder report (head 8e25e107): tasks 1-5 done. cargo test --workspace with PostgreSQL 3367; cicada-containers unit 104; 5 new ContainerDockerTest steps pass; docs 11; fmt, clippy -D warnings, openspec strict clean; single-feature builds pass; Cargo.lock adds no crate (tokio-util as a dependency of cicada-containers); gate.py full and e2e pass of 8e25e107.
Departures: allow(dead_code) on paths.rs kept (the default copy_out doesn't use the module; place is only used by backends), and the new copy module has the same allow; path_parts takes an action word for its error text; deepest_mount pulled out of place for the fake; the reader fails the call if a stream error is stored though tar returned success; PAX global headers are passed over and not counted.
Docs: handed off, nothing blocks.
