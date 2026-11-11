# Tasks

The numbers in brackets are the decisions of `design.md`.

## 1. TLS in the build

- [x] 1.1 `crates/cicada/Cargo.toml`: the `postgres` feature also names `sqlx/tls-rustls-ring-native-roots` [1, 2,
      3]. Verify: `cargo tree -p cicada --no-default-features --features postgres -e normal` lists `rustls`; the
      same with `--features sqlite` lists no `rustls` and no `native-tls` (scenario "A SQLite host"); the two CI
      steps for the feature builds pass locally as the workflow writes them.
- [x] 1.2 A test that opens a store on `CICADA_PG_URL` with `sslmode=require` and expects the error of a server
      without TLS [4]; it is skipped without the URL. Verify: with PostgreSQL up it passes (scenario "TLS is asked
      from a server that offers none"), and it fails with "built without TLS support" when the change of 1.1 is
      taken out in a scratch change (say in the report how you checked). The existing PostgreSQL tests still pass
      (scenario "A URL without a mode").

## 2. Docs and checks

- [x] 2.1 `docs/operations.md`: a paragraph on TLS for PostgreSQL [6]; `README.md`: a pointer to it where it shows
      how to open PostgreSQL; `CHANGELOG.md`: one entry under Fixed, with the sentence on the default mode. Verify:
      `cargo test -p cicada-examples --doc` passes.
- [ ] 2.2 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with
      `CICADA_PG_URL` set, and `openspec validate postgres-tls --strict`. Verify: all pass; if PostgreSQL cannot be
      started, say so in the report and leave this task open.
