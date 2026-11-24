---
id: listening-line-race.code.1
kind: code
change: listening-line-race
round: 1
base: 82dbac25e5e7f540fd662c4a3ff5ee2a3ce3c5a5
head: 3cee950398cb8a1e73e7cf62af323d026785d7ef
requested: 2026-11-24T09:18:10+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done listening-line-race.code.1 <file>`.

## Note from the requester

Version: none

Builder report: tasks 1-3 done. Both helpers (sample-service/tests/sample_service.rs start_as, crates/cicada-app/tests/common/mod.rs serve) search only the output up to its last newline. cicada-app --all-features 188 passed x3; cicada-sample-service 38 passed x3; fmt, clippy clean; openspec valid; gate.py run full pass and --e2e pass for 3cee950. No departures. Note: helpers still from_utf8(...).unwrap() the buffer (pre-existing, ASCII line).
