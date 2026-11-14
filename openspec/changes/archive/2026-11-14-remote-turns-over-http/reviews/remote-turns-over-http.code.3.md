---
id: remote-turns-over-http.code.3
kind: code
change: remote-turns-over-http
round: 3
base: b14f151b0a642fd506963600040d694622b34389
head: 448610f1b3400ca75ebc80c85ac1470e1471b9c9
previous: b67268f2d3f7011b6e33634d1a573d67911fa4f3
requested: 2026-11-14T06:51:49+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done remote-turns-over-http.code.3 <file>`.

## Note from the requester

Round 3 (owner allowed one more fix round). Fixes the round 2 blocker only, in commit 448610f.

- crates/cicada/src/api/leases.rs: each record is declared once in a `record!` macro. The public type derives Serialize and JsonSchema only; a private shadow (`<Name>Wire`) derives the field decoder with `#[serde(remote = "<Name>")]`; the public `Deserialize` impl runs that private decoder through the object-only check. No public inherent `deserialize` remains.
- New regression `every_record_called_directly_refuses_a_positional_array_and_takes_its_object`: for all 18 records, calls `Type::deserialize` directly on a serde_json::Deserializer and on a serde_json::Value; array refused, object taken.
- Wire format and docs/openapi.json unchanged.

Please check that the round 2 finding is closed and the fix adds no new problem. Everything else was confirmed in round 2.
