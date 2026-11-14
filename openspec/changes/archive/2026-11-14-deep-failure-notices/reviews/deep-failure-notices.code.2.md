---
id: deep-failure-notices.code.2
kind: code
change: deep-failure-notices
round: 2
base: c54153b4edbf1d7b215e9a623a4b3a7428d03a17
head: 38baea2e3d0590b9af899527f325e55fd53914a6
previous: 68e51c375fdfc28878351ebe8bd990fa7eb00bea
requested: 2026-11-14T08:56:07+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done deep-failure-notices.code.2 <file>`.

## Note from the requester

Version: 0.17.0

Round 2: fixes the round 1 blocker in commit 38baea2 (examples/src/dead_letters_upgrade.rs).

- New `check(&Export)` refuses before any deletion: a NUL in any string or key at any depth; a state deeper than 64 levels; a state that does not decode as the 0.17.0 state type (also refuses null); an address that appears twice in the export (added: restore would silently skip the second create, then fail its compare).
- `convert` ends by calling `check`; `reset` runs `check` again as its first step, before it reads or deletes anything, so no call order can delete actors for an invalid export.
- Operations guide, "Upgrading to 0.17.0" steps 5 and 6, say what `check` covers and that `reset` repeats it.
- Reuse: the core has no public validate-without-write entry point (its check is private to `cicada`); a dry run on an in-memory store would need `cicada`'s `testing` feature as a normal dependency of the example. So `check` is a narrow local copy of what `create_actor` checks on a hosting node; its doc comment says so.
- Schema: `restore` uses `create_actor`, which does not check the contract's JSON Schema, so `check` does not either; the existing dead-letters test shows the schema and decoder agree on the letter fields.
- Regression `a_state_with_a_nul_character_is_refused_before_anything_is_deleted`: NUL in a value and in a key; `convert`, `check` and `reset` each refuse; after the refused `reset` a fresh export equals the earlier one, and registering the 0.16 contract still succeeds (the old registration was never replaced).

Checks: fmt and clippy -D warnings clean; cargo test --workspace with PostgreSQL 2740 passed, 0 failed.
