# Docs handoff for lease-limit-docs

Version: 0.18.11. Refresh the dependency pins of `docs/how-to/install.md` once it is published. No API, limit,
validation, wire format, contract hash, store schema, guarantee, CLI or HTTP route changes.

## What changed

None for the handwritten docs. The change corrects generated descriptions only:

- The rustdoc of `LeaseTurnsRequest` (`crates/cicada/src/api/leases.rs`) and the regenerated `docs/openapi.json` now
  say "1 to 100 types (`MAX_LEASE_TYPES`)" for `types` and "1 to 100 (`MAX_LEASE_BATCH`)" for `limit`, instead of
  1 to 64 types and 1 to 256 turns. The core already refused more than 100 of either; nothing about that changes.
- The test `the_lease_request_documents_the_limits_the_core_checks` in `crates/cicada-axum/tests/openapi.rs` compares
  the generated `owner`, `types` and `limit` descriptions with `MAX_ID_LENGTH`, `MAX_LEASE_TYPES` and
  `MAX_LEASE_BATCH`.
- The audit found no other mismatch: the renew, release and complete bodies, the core guide's remote lease section
  (already 1–100 for both counts), the `turn-leases` spec and ADR 0039 agree with the code.

Why no prose: the core guide already states the correct limits, and no guide, guarantee row, doctest, link or skill
route names the corrected sentences or the new test.

## Blocking

None. Every doctest, link, guarantee row and skill route passes without prose edits.

## Status

Red: `cargo test -p cicada-axum --test openapi` before the correction failed the new test on `types` ("1 to 64 types").
Green: after the correction and `CICADA_UPDATE_OPENAPI=true`, the same run without the variable passed (10 passed). The
generated diff of `docs/openapi.json` holds the two descriptions and the version only. Run directly on c915092 with
PostgreSQL on port 55432: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` (clean);
`cargo test --workspace` (2947 passed, 0 failed); `cargo test -p cicada-app` (149 passed); the script tests (64 passed);
`scripts/ci/feature-builds.sh` and `scripts/ci/dependency-trees.sh` (pass); the Docker E2E suite (1 passed); `openspec
validate lease-limit-docs --strict` (valid). After the owner's recovery of an earlier, interrupted gate run, `gate.py
run` and `gate.py run --e2e` passed on 9692abb (full and e2e evidence for tree 4033b02f3ebc). Nothing is written in
docs-owned files, and no owner fix is integrated.
