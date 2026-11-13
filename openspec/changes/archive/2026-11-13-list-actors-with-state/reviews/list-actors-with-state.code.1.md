---
id: list-actors-with-state.code.1
kind: code
change: list-actors-with-state
round: 1
base: 3fd060d008d8f6a780c2d712e47eb7e0f1c4c6d5
head: cb03a219861a4cb9edb74d0ccc1f1981ee8d1b51
requested: 2026-11-13T16:02:27+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done list-actors-with-state.code.1 <file>`.

## Note from the requester

Version: 0.16.9

Builder report (round 1)

list-actors-with-state is built as 0.16.9 on branch feat/list-actors-with-state, five commits on top of main (3fd060d). All five tasks done, every check passed, PostgreSQL included.

Tasks
1. Done. The private Backend::list_records in all three stores: SQL stores run one SELECT with named columns per page; memory takes one lock. It reuses the prefix, cursor and row-decoding code of list and get. Three new conformance cases; the S2 and S3 rows of docs/guarantees.md now name the new S3 test.
2. Done. ActorSystem::list_actors_with_state, sharing a private snapshot helper with get_actor. Six new runtime tests, one of them a legacy 100-level state.
3. Done. GET /actors/states on both routers with operationId listActorStates. ActorClient::list_actors_with_state; docs/openapi.json regenerated.
4. Done. The core and HTTP guides each have a section with a compiling example. Endpoint table, depth paragraph (67 levels), crate docs, architecture, changelog and version updated.
5. Done. Finishing checks.

Checks
- cargo fmt --all --check: passed.
- cargo clippy --workspace --all-targets -- -D warnings: passed.
- cargo test --workspace with PostgreSQL: 2360 passed, 0 failed (173 PostgreSQL conformance tests).
- cargo check -p cicada with no feature, and with sqlite, postgres and testing each alone: passed.
- openspec validate --strict: passed.
- Not run: the E2E suite in Docker.

Departures and open questions
- The fuzz fault store gained a Method::ListRecords variant, so Method::ALL now has 36 entries.
- The HTTP 500 for a corrupt row is not tested end to end. A SQLite store test shows the whole page fails as corrupt, and an existing unit test maps corrupt data to 500.
- No open questions.

New concepts
- State page: one store read returns a page of full actor snapshots. All rows in a page come from one committed snapshot, each state with its own version.
- No snapshot across pages: a later page shows later commits; it misses actors created before its cursor and leaves out deleted ones.

Breaking changes: none. The new client method needs a server on 0.16.9 or later; against an older server it returns that server's error.
