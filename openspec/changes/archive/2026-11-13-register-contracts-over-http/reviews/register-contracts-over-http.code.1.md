---
id: register-contracts-over-http.code.1
kind: code
change: register-contracts-over-http
round: 1
base: 816a99109179e1a14b1a77f650a3d78f15b19739
head: 96810bc3117ff1e11461716eb643ff27908c213f
requested: 2026-11-13T18:03:22+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done register-contracts-over-http.code.1 <file>`.

## Note from the requester

Version: 0.16.12

Builder report (round 1)

register-contracts-over-http (0.16.12) is built and every finishing check passes. Branch feat/register-contracts-over-http, 5 commits on top of 816a991. Nothing pushed, merged or tagged.

Tasks
- Task 1, core (ActorSystem::register_contract, the checks, the cache fix): done. The cache fix was tested by putting the old cache logic back: the peer delete/re-register test then fails.
- Task 2, PUT /types/{name}, its OpenAPI entry and docs/openapi.json: done.
- Task 3, ActorClient::register_type, and the documented-routes test updated: done.
- Task 4, docs, version and changelog: done. ADR 0036 and its index row were already there.
- Task 5, finishing checks: done.

Checks
- cargo fmt --all --check: passed.
- cargo clippy --workspace --all-targets -- -D warnings: no warnings.
- cargo test --workspace with PostgreSQL: 2485 passed, 0 failed, including the 120-level round trip on SQLite and PostgreSQL after reopening the store.
- openspec validate register-contracts-over-http --strict: valid.
- cargo check -p cicada with no feature and with each feature alone: passed.
- Not run: the Docker end-to-end tests.

Departures and open questions
- On the read-only router, PUT gets axum's plain 405 (empty body, Allow: GET,HEAD), not a problem+json body; matches the design's "ordinary 405", and the test checks exactly that.
- The HTTP handler builds the contract with Contract::new, which computes a hash before the core checks run. Safe because the JSON parser stops at 128 levels; the core checks first, then computes the hash again.
- No open questions.

New concepts
- Registering without hosting: a server records a type's contract without running its handler. Creates and sends are accepted and checked against the schemas at once; the messages wait until a node that hosts the type claims them.
- MAX_SCHEMA_DEPTH (120): a depth limit for schemas registered this way, separate from the 64-level limit on values; keeps the stored contract and type views readable by JSON parsers that allow 128 levels.
- contract-conflict (409) and invalid-contract (422): two new problem types. The first registration of a name wins; a different hash is refused and nothing is replaced.

Breaking changes: none.
