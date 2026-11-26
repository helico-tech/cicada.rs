---
id: placement-identity-lock.code.1
kind: code
change: placement-identity-lock
round: 1
base: f7a4d5b2abb411561697825b040794c331ce542f
head: 2bc7b45bfad40df41553a57709f896eeacfd1463
requested: 2026-11-26T19:34:51+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done placement-identity-lock.code.1 <file>`.

## Note from the requester

Version: 0.25.0 (opens the batch; a pool contract change and a NodeConfig field)

Builder report (head 2bc7b45b; gate full pass of 673d9dfd + docs pass for the tick): tasks 1-5 done. NodeConfig::hostname
and ActorSystem::hostname(); the identity lock with IdentityErrorKind::{Invalid, Locked}; the duplicate check, the pool's
Duplicate message and the mark's host; contract reference and samples regenerated; 0.25.0 section. Checks: workspace
with PostgreSQL 3708, fmt, clippy -D warnings, openspec --strict, examples docs 11 / doc 51 / skill 18. Gap-fills to
judge: "no new places" only for marks with a host (instance marks as ADR 0069); a Duplicate for a silent node changes
nothing; a report-only mark clears at the first pool turn after its silence time; NodeConfig::hostname bounds like label.
Docs: handed off, nothing blocks.
