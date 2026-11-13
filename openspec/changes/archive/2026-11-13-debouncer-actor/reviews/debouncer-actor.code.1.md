---
id: debouncer-actor.code.1
kind: code
change: debouncer-actor
round: 1
base: 16cc667671a280bdfa4836ca7a69d4672445ba84
head: 13743afde864a582526eb479bce9cee35062391c
requested: 2026-11-13T10:29:36+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done debouncer-actor.code.1 <file>`.

## Note from the requester

Version: 0.16.3

Builder report (round 1)

All five tasks of debouncer-actor are done on branch feat/debouncer-actor, at version 0.16.3. Nothing pushed, merged or tagged.

Tasks
1. Done: crates/cicada-actors/src/debouncer.rs adds the actor, its messages, its checked state, the typed submit helper and the exports, with unit tests and a compile-fail doctest.
2. Done: tests/debouncer.rs has 23 virtual-time tests, one or more per spec scenario. tests/store_failure.rs adds 2 SQLite tests: a flush whose store read fails and is retried, and a Submit that cancels that retry.
3. Done: contract pinned in docs/reference/contracts/cicada.debouncer.json and checked against samples in wire_formats.rs. The sample service hosts the type with no instance; snapshot, text and state sample added. No existing contract changed.
4. Done: examples/src/debounce.rs (Example 11) has 4 tests, plus a "Debouncer" section in the actors guide with a doctest. Tables, README, architecture, AGENTS map, crate description, changelog and OpenAPI version updated.
   Addition asked by the build loop (from the note of the rate-limiter-actor review): README and core guide now say "twelve worked examples", and the core guide's Examples table has rate_limit and debounce rows.
5. Done: results recorded in tasks.md.

Checks
- cargo fmt --all --check: clean.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test --workspace with PostgreSQL: 2149 passed, 0 failed.
- openspec validate debouncer-actor --strict: valid.
- Not run: the Docker end-to-end suite (CICADA_E2E=1).

Departures and open questions
No departures from the design. One finding outside the change: when the 62-level Submit fails, its Failed notice holds that payload one level deeper, past the 64-level limit, so the watcher's turn fails when it reads the notice (dead letters would fail the same way). The depth test reads the cause from the watcher's failed envelope instead. The build loop takes this to the owner as a separate change; it is not for this review to fix.

New concepts
- Debouncer (cicada.debouncer): keeps only the latest submission of a burst; sends it after a quiet period, at the latest a fixed longest wait after the burst's first submission.
- Public Flush: sends what is kept now and clears it in the same commit; with nothing kept it does nothing. A retried or redriven old Flush sends the current burst.

Breaking changes: none.
