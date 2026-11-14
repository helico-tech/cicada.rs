---
id: deep-failure-notices.code.1
kind: code
change: deep-failure-notices
round: 1
base: c54153b4edbf1d7b215e9a623a4b3a7428d03a17
head: 68e51c375fdfc28878351ebe8bd990fa7eb00bea
requested: 2026-11-14T08:46:17+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done deep-failure-notices.code.1 <file>`.

## Note from the requester

Version: 0.17.0 (breaking, ADR 0043)

Builder's report:

Tasks (all done):
1. serde_json float_roundtrip on for every crate. Store tests (memory, SQLite, PostgreSQL): a Failed notice for a 64-level payload is written and claimed whole; numbers keep their bits (PostgreSQL turns -0.0 into 0.0, pinned by a test). A watcher reads the notice on a node and in a worker over HTTP. Without the feature the new test fails on 1.0715660391465826e-75.
2. Dead letters keep a payload deeper than 61 levels as JSON text in `payloadJson`. A redrive that cannot parse or send the text records why and sends nothing.
3. Example 16 (examples/src/dead_letters_upgrade.rs): the tested upgrade export, convert, reset, restore.
4. Batch boundary test, JournalEntry::Sleep comment, event Unsubscribe test, debouncer comment.
5. Guides, architecture, operations ("Upgrading to 0.17.0"), guarantees table, changelog, OpenAPI version.
6. Finishing checks.

Checks: cargo test --workspace with PostgreSQL 2739 passed, 0 failed; fmt --check, clippy -D warnings, openspec validate --strict pass; cicada with no feature and each feature alone; cicada-app with no feature and with tui (not checked: cicada-app sqlite or postgres alone). A throwaway consumer crate outside the repo parses the regression float exactly. Only the dead-letters contract hash changed; flow fixture edits are description only. Not run: Docker E2E.

Departures:
- Also fixed the same wrong comment in batcher.rs (said the witness cannot read a deep notice).
- Added axum and tower as dev-dependencies of cicada-actors for the HTTP 422 test.
- Copied the 0.16 dead-letters contract to examples/tests/fixtures/ so the upgrade test starts from a store as 0.16 left it.

New concepts: payloadJson / LetterPayload (a letter holds payload or payloadJson, never both; DeadLetter::payload is LetterPayload::Value or ::Json; value() parses lazily so reading a state never fails on it). Exact float parsing (a database may still change a number first: PostgreSQL -0.0 -> 0.0).

Breaking: DeadLetter::payload is a LetterPayload, not a Value; the dead-letters contract hash changed (a 0.17.0 node hosting dead letters won't start against the 0.16 registration; maintenance steps in the operations guide); 0.16 readers cannot decode a letter with payloadJson.
