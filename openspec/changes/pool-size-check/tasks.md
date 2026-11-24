# Tasks

Normal lane, a patch: the change joins the unreleased batch in `CHANGELOG.md` (0.21.0 once `container-limits-and-kinds`
has landed; builder.md, Version: add entries under the existing unreleased heading, no new version). Builds on
`container-limits-and-kinds`. Follow `design.md`; the specs are `specs/store-backends/spec.md` and `specs/app/spec.md`.
No public API, wire format, schema, guarantee or command-line change; `cicada-app` gets no code change.

- [x] 1. **The pool through the private trait.** In `crates/cicada/src/store/mod.rs`: `fn pool_size(&self) ->
  Option<u32>` on `Backend` with a default `None` and a doc comment (decision 3). The PostgreSQL backend
  (`crates/cicada/src/store/postgres/`) returns its `max_pool_size`; `Store` in `store/handle.rs` delegates, as for
  `wake_ups`; SQLite and the in-memory store keep the default. Verify: `cargo check -p cicada --features postgres`,
  `cargo check -p cicada --features sqlite`, `cargo check -p cicada --features testing`.
- [x] 2. **The rule and the warning.** In `crates/cicada/src/runtime/system.rs`: a private
  `pool_warning(pool: Option<u32>, concurrency: usize) -> Option<String>` with the rule `pool < concurrency + 4`
  without overflow and the text of decision 4 (both numbers, the number needed, `PostgresOptions::max_pool_size`,
  `--store-pool-size` / `CICADA_STORE_POOL_SIZE` and the concurrency). `ActorSystem::start` calls it with
  `store.pool_size()` and `config.concurrency` once, only when the node hosts at least one type, and logs the
  result with `tracing::warn!` before it spawns anything (not in `initialize`). Verify: unit tests of `pool_warning`
  (`None` never warns; `concurrency + 4` exactly does not; one less does and the text holds both numbers and the
  setting names; `usize::MAX` concurrency does not overflow); `cargo test -p cicada --lib`.
- [x] 3. **The warning at start, on PostgreSQL.** Add `tracing-subscriber` (workspace) to `cicada`'s
  dev-dependencies. In `crates/cicada/src/tests/postgres/runtime.rs` (skipped without `CICADA_PG_URL`): a node with
  concurrency 4 hosting one type on a pool of 6 logs one warning naming 6, 4 and 8 and starts; at a pool of 8, and
  a node hosting no types on a pool of 1, it logs none. Capture with a thread-local test subscriber
  (`tracing::subscriber::set_default` with a writer into a shared buffer) on a single-thread runtime
  (`#[tokio::test]`, not `multi_thread`): the subscriber sees only its own thread, which works because `start` logs
  the warning before it spawns anything. Verify: `CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada
  cargo test -p cicada --lib tests::postgres`.
- [x] 4. **Changelog and docs handoff.** In `CHANGELOG.md` under the unreleased heading, Changed: a node on
  PostgreSQL warns at start when its pool is smaller than its concurrency plus 4. `docs-handoff.md` (template in
  `.claude/skills/change-loop/builder.md`) carries the warning, its text and when it fires (start only, nodes that
  host types, PostgreSQL only), and the limit that nodes or a local `Worker` sharing one `Store` are not summed, for:
  `docs/reference/stores.md` (the `max_pool_size` row), the sizing table of `docs/operations.md` (the PostgreSQL
  pool row) and `docs/guide/app.md` (the `--store-pool-size` row: a smaller given pool is kept and warns); nothing
  blocks. No edit to `README.md`, `AGENTS.md` or the handwritten docs. Verify: `cargo test -p cicada-examples --test
  docs`; `cargo test -p cicada-examples --test skill`.
- [ ] 5. **Finishing checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace`, `openspec validate pool-size-check --strict`, commit, then `python3 scripts/gate.py run`
  (no E2E: the change touches the core's start, not `sample-service/`, `e2e/`, the serve or worker code of
  `cicada-app` or the HTTP crates; E2E runs at the release).
