# Tasks

Normal lane, version 0.26.0 (unreleased). Builds after `tray-settings` (shares `cicada-app`; rebase onto it when it
landed first). Follow `design.md`; the spec is `specs/store-counts/spec.md`. No ADR.

- [x] 1. **The count in the store.** `StoreStats::failed` (`#[non_exhaustive]`), counted by the outcome listing's
  visibility rule in the stats read of PostgreSQL, SQLite and the memory store (decisions 1 and 2). Verify:
  conformance cases for each scenario, plus a `Keep::For` shorter than the retention and a `Keep::Last` after trimming
  not counted; `cargo test -p cicada` with `CICADA_PG_URL` set.
- [x] 2. **The wire.** `StatsView::failed: Option<u64>` (`#[non_exhaustive]`), mapped from the store; struct literals
  in `examples/`, `sample-service/` and other crates updated; `docs/openapi.json` regenerated with
  `CICADA_UPDATE_OPENAPI=true` (decision 3). Verify: the HTTP test of the field and `cargo test -p cicada-axum`.
- [x] 3. **Client and command line.** An answer without `failed` decodes to `None`; `cicada stats` shows the number,
  `-` for `None`; `docs/reference/cli.md` regenerated if its help changed (decision 4). Verify: `cargo test -p
  cicada-client` and `cargo test -p cicada-app`.
- [x] 4. **Changelog and docs handoff.** The 0.26.0 section names `failed` under Added and the two structs under its
  BREAKING line; `docs-handoff.md` carries the design's Docs section. No edit to `README.md`, `AGENTS.md` or the
  handwritten docs. Verify: `cargo test -p cicada-examples --test docs` and `--doc`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace` with PostgreSQL, `openspec validate stats-failed-count --strict`, commit. The change loop
  runs the gate at landing.
