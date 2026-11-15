# Tasks

- [x] 1. Fixed canonical and contract vectors. Add `fixtures/canonical.json` and `fixtures/contracts.json` with
  format 1, stable case names, string inputs and the recorded baseline. Add public-API tests in
  `crates/cicada/tests/fixtures.rs` for the cases in the design: exact canonical output, idempotence, prefixed hashes,
  parser refusals and schema-description traversal. Assert expected contract text is already canonical, hash its
  exact UTF-8 bytes, and compare the actual contract hash. Keep the parser/formatter and dependencies unchanged;
  no expectation-update switch. Keep ADR 0047 and its index consistent with the built data.
  Verify: `cargo test -p cicada --test fixtures`, `cargo test -p cicada --lib contract`, and existing contract
  snapshot tests (`cargo test -p cicada-test`, `cargo test -p cicada-actors --test wire_formats`) without update
  variables. Temporarily mutate one expected digit and one hash, record the failures, and restore before commit.

- [x] 2. Lease examples. Add `fixtures/leases.json` and extend `fixtures.rs` with valid, accepted/normalized and
  invalid decode/conversion cases as designed. Include every result/report/state/outcome/control variant using
  exhaustive matches and required-case assertions, effects/policies, exact i64 extremes, nullable fields and
  duration/delivery boundaries. Use fixed byte output, not numeric `Value` equality; document in the data that
  serializer output is an example rather than a field-order rule for ports. Verify: `cargo test -p cicada --test
  fixtures`, plus existing wire conversion tests in `cargo test -p cicada --lib api`.

- [x] 3. Status data and completeness checks. Add `fixtures/status.json`; test the actual private `unexpected()`
  in `crates/cicada-client/src/turns.rs` for statuses 100–599 without sockets, including misleading problem status.
  Adapt the scripted tests in `crates/cicada-client/tests/turn_source.rs` to consume success and registration rows,
  including malformed success and a vanished conflicting registration. In `crates/cicada-axum/tests/openapi.rs`,
  compare success rows with the route responses. Finish format/file membership, unique-name and reference checks
  across all four files. Verify: `cargo test -p cicada-client --lib turns`, `cargo test -p cicada-client --test
  turn_source`, `cargo test -p cicada-axum --test openapi`, and `cargo test -p cicada --test fixtures`.

- [x] 4. Package and publish the second asset. Add `scripts/package-fixtures.py`, sharing deterministic archive
  helpers with `package-skill.py` through an explicitly loaded helper module. Preserve the skill CLI and bytes.
  Read only exact tag blobs; validate version, fixture formats, regular modes and exact membership, and verify
  read-back. Extend `gate.py` to verify both archives before tag push, atomically retain the complete digest map,
  refuse legacy singular-digest state, and resume only matching subset drafts by uploading missing assets.
  Add tests in `scripts/test_*.py` and extend `examples/tests/skill/test_package_skill.py` as needed: isolated
  `python3 -I -B` calls, unchanged skill bytes, deterministic fixtures, dirty-worktree independence, malformed
  members/formats/modes, failed second package with no new tag push, changed saved digests/name set, legacy-state
  refusal, partial draft, uncertain upload, foreign/duplicate assets, and untouched published releases.
  Verify: `python3 -m unittest discover -s scripts -p 'test_*.py'` and
  `cargo test -p cicada-examples --test skill`. Use temporary repositories and fake GitHub; never publish from tests.

- [x] 5. Version and handoff. Apply the build loop's next patch version in Cargo.toml/Cargo.lock, the consumer
  skill metadata and generated OpenAPI version. Add the changelog and `docs-handoff.md` for architecture's canonical
  rule, HTTP port guidance, the release procedure's two archives, AGENTS map and dependency pins. State the tested
  dependency/feature limits and absence of historical backfill. No handwritten documentation edits or new guide
  route; record `Blocking: none` unless verification finds a real blocker. Verify:
  `cargo test -p cicada-examples --test skill --test docs` and `cargo test -p cicada-examples --doc`.

- [x] 6. Finishing. Run `openspec validate conformance-fixtures --strict`. Commit the final tree, then run
  `python3 scripts/gate.py run --e2e` and report `python3 scripts/gate.py status`. The full policy includes
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace` with PostgreSQL,
  plus feature, documentation, script and Docker E2E checks. Confirm contract snapshots pass without regeneration
  and report the mutation checks. Packaging tests are not evidence of a live GitHub release. Do not install hooks,
  push, tag or publish from the builder; the owner-approved build loop releases later.
