# Tasks

## 1. Contract hash without descriptions

- [ ] 1.1 In `crates/cicada/src/contract.rs`, hash a copy of the contract JSON whose state and message schemas have
      every `description` keyword removed, walking only the keywords that hold subschemas (design, decision 2). Keep
      `to_json`, `canonical_json` and `canonical_sha256` as they are, and update the doc comments of `Contract` and
      `Contract::hash`. Verify: unit tests in `contract.rs` show that a changed description (top level, in a property of
      a property, in `items`, in `anyOf`, in `$defs`) keeps the hash; that a property, a `$defs` entry and a message
      type named `description` still count; that a `description` key inside `default` still counts; and that `to_json`
      keeps every description.
- [ ] 1.2 In `crates/cicada-test/tests/startup.rs`, add two actor types with one type name whose message and state types
      differ only in doc comments, and one whose message has a property named `description`. Verify: a node with the
      second type starts on a store where the first one is registered; the stored contract keeps the first one's
      descriptions; the type with the extra `description` property fails start with a different contract.
- [ ] 1.3 Write ADR 0017 (contract hash without descriptions, published with them; the SQLite store refuses a foreign
      database) and update `docs/architecture.md` (domain language: Contract; the store section). Link the ADR from the
      README's decisions table and change the README's "Contract changes" row and the contract notes in
      `docs/operations.md`. Verify: the links resolve, and `cargo test -p cicada-examples --doc` passes.

## 2. The SQLite store refuses a foreign database

- [ ] 2.1 In `crates/cicada-store-sqlite/src/schema.rs`, pass the path to `migrate` and, when `user_version` is 0 and
      `sqlite_master` has a table whose name does not start with `sqlite_`, return `StoreError::Invalid` naming the path.
      Update the module doc. Verify: the tests of 2.3.
- [ ] 2.2 In `crates/cicada-store-sqlite/src/store.rs`, connect the writer without a journal mode and set WAL after the
      migration succeeded; update the doc of `SqliteStore::open` and of the crate. Verify:
      `database_uses_wal_and_has_its_tables` still passes, and the SQLite conformance tests pass.
- [ ] 2.3 In `crates/cicada-store-sqlite/tests/sqlite_store.rs`, test a file with a foreign table (refused with the
      message, file bytes unchanged), a file of zero bytes (set up, WAL) and a new path (set up, WAL). Verify:
      `cargo test -p cicada-store-sqlite`.

## 3. Release notes and version

- [ ] 3.1 Add `## 0.3.0 — unreleased` at the top of `CHANGELOG.md`: what changed, and that nodes of 0.2.0 and 0.3.0
      must not share a database. Verify: the section reads in the file's style and `scripts/release-check.sh v0.3.0`
      fails only because the section is not dated (and the tag is not on `main`).
- [ ] 3.2 Set the workspace version in `Cargo.toml` to `0.3.0` and run `cargo check` to update `Cargo.lock`. Check the
      README and docs for mentions of the version or tag that must change. Verify: `grep` for `0.2.0` and `v0.2` shows
      only mentions that must stay (the README pins the released tag until 0.3.0 is tagged).

## 4. Finish

- [ ] 4.1 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace` and `openspec validate contract-descriptions-and-foreign-sqlite --strict`. Verify: all
      pass.
- [ ] 4.2 With PostgreSQL up (`docker compose up -d postgres`), run
      `CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada-store-postgres`. Verify: all
      pass, or the report says why they could not run.

## Workflow follow-up

- Archive the change after review.
- Tag `v0.3.0` when the owner asks (docs/operations.md, Releasing), and then move the README's pinned tag to it.
