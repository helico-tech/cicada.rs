# Tasks

- [ ] 1. Add `the_lease_request_documents_the_limits_the_core_checks` in
  `crates/cicada-axum/tests/openapi.rs`. Assert the generated `LeaseTurnsRequest` descriptions against
  `MAX_LEASE_TYPES`, `MAX_LEASE_BATCH` and `limits::MAX_ID_LENGTH`, with number boundaries and useful diagnostics.
  Verify: run `cargo test -p cicada-axum --test openapi` before correcting the text and record the expected failure
  on the current lease descriptions. Keep the existing tests that pin and enforce the runtime limits unchanged.
  Commit this regression test with the correction in task 2 so the commit passes its tests.

- [ ] 2. Correct the two rustdoc descriptions in `crates/cicada/src/api/leases.rs` to 1–100, naming the respective
  constants in plain code spans, and regenerate `docs/openapi.json`. Leave validation and schema keywords unchanged.
  Use the build loop's next patch version: update Cargo.toml/Cargo.lock, the consumer skill version and CHANGELOG
  through the normal version procedure. Write `docs-handoff.md` with the version and no blocking prose work: the
  core guide already has the right limits; ask the docs session to refresh release pins as usual.
  Verify: `CICADA_UPDATE_OPENAPI=true cargo test -p cicada-axum --test openapi`, then
  `cargo test -p cicada-axum --test openapi` without the variable. Inspect the generated diff: only descriptions
  and normal version metadata change. No handwritten documentation edits.

- [ ] 3. Finishing checks. Run `openspec validate lease-limit-docs --strict`. Commit the final tree, then run
  `python3 scripts/gate.py run --e2e` and report `python3 scripts/gate.py status`. The full gate includes
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace` with PostgreSQL,
  plus the feature, docs, skill and Docker E2E checks. Record the red/green regression evidence and any blocked
  check; do not tick this task without passing evidence. Do not install hooks, push, tag or release from the builder.
