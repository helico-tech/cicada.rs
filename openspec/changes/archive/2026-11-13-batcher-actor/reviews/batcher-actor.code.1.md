---
id: batcher-actor.code.1
kind: code
change: batcher-actor
round: 1
base: 2909ea8faa8161a18d3e2665cdc1be6b4543ad0b
head: b9cf305da36b6c36e9bf24418390d5f5fd79505a
requested: 2026-11-13T13:12:31+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done batcher-actor.code.1 <file>`.

## Note from the requester

Version: 0.16.4

Builder report (round 1)

batcher-actor is built: all five tasks done, every finishing check passes. Branch feat/batcher-actor, clean, 5 commits on top of main (2909ea8 = origin/main). Two builders worked on it: the first stopped on an API error after tasks 1–3 and part of task 4; the second checked that work against the design, kept it and finished. Nothing pushed, merged, tagged or archived.

Tasks
- 1–3: done (65956a0 the actor, cf6a9e7 the tests, 7b44ce3 the contract and the sample service); they match the design.
- 4: done in 825c71d. The first builder's example (examples/src/batching.rs, examples/tests/batching.rs) and table edits kept; added the "Batcher" section of docs/guide/actors.md with its doctest and HTTP calls, the count of thirteen examples with batching in the core guide's Examples table, the changelog under 0.16.4 — unreleased, version 0.16.4 with Cargo.lock, and the regenerated docs/openapi.json.
- 5: done in b9cf305.

Checks
- cargo fmt --all --check: passed.
- cargo clippy --workspace --all-targets -- -D warnings: no warnings.
- cargo test --workspace with PostgreSQL: 2189 passed, 0 failed (incl. 36 doctests of cicada-examples, 11 docs tests, 4 batching example tests).
- openspec validate batcher-actor --strict: valid.
- Not run: the Docker E2E tests.

Departures from the design: none. No open questions.

New concepts
- Batcher: collects items in the order it handles them and sends them to one fixed target as {"items": [...]}; sends when the batch is full, at maxWait after the first item, or on a public Flush.
- Inline flush: the Add that fills the batch, or arrives after the deadline, sends the batch in its own turn, so a stored batch always holds fewer than maxItems items. If that send fails, the Add keeps no item, and its failure (or dead letters) holds it until a retry or redrive.

Breaking changes: none.
