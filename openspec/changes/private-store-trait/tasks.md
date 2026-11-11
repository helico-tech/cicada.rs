# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. The order keeps the trait public until every test that needs it is inside the core.

## 1. Count, and prepare what needs no move

- [x] 1.1 Before anything moves, count the tests of each suite that will move: the conformance cases per backend,
      `cicada-fuzz`, `cicada-store-sqlite`, `cicada-store-postgres` (with `CICADA_PG_URL` set), the 11 test files of
      `cicada-test` that need the trait, and `crates/cicada/tests/store_support.rs` and `retry_policy.rs`. Verify: the
      numbers are in the report, with the commands that gave them (`cargo test -p … -- --list`).
- [x] 1.2 `ActorSystem::now()` and `ActorSystem::next_due_at()` [12]. Verify: tests on virtual time with
      `TestActors` for the scenarios "The store's time", "A message that is due later", "Nothing pending" and "A
      paused actor".
- [x] 1.3 A conformance case for "a closed store fails every operation as unavailable" [6]. Verify: it passes for
      the in-memory store, SQLite and, with `CICADA_PG_URL` set, PostgreSQL (scenario "Every backend").
- [x] 1.4 `cicada-axum` without `FlakyStore` [6, 11]: the `503` tests close the store; a panic and corrupt data as
      `500` become unit tests of the error mapping and of the wrapper that catches a panic; `store.list` becomes
      `system.list_actors`. Verify: tests for the scenarios "A request after the store is closed" and "Health after
      the store is closed", and the two unit tests, pass.
- [x] 1.5 A test of `ActorSystem::health` on the fault wrapper: unhealthy while the store fails, healthy again when
      it is back [6]. Verify: the test passes.

## 2. The stores and the harness move into `cicada`

- [x] 2.1 `crates/cicada/Cargo.toml`: the features `sqlite`, `postgres` and `testing`, none on by default; sqlx as
      an optional dependency with only the part each feature needs; `cicada` as a dev-dependency of itself with
      the three features [3, 10, 11]. Verify: `cargo check -p cicada` passes with no feature, with `sqlite`, with
      `postgres` and with `testing`, each alone.
- [x] 2.2 Move the code of `cicada-store-sqlite` and `cicada-store-postgres` into `cicada::store`, each behind its
      feature, and `InMemoryStore`, `TokioClock`, `VirtualClock`, `TestActors` and its builder into
      `cicada::testing` behind `testing` [1, 2, 7, 10]. `cicada-test` turns `testing` on and exports
      `TestActors`, `TestActorsBuilder`, `TokioClock`, `VirtualClock` and `default_start` under the names they have
      today; the snapshot helpers stay in `cicada-test`. Verify: `cargo test --workspace` passes, and a test of a
      crate that names no backend feature (`cicada-extras`) runs on `TestActors` with unchanged imports (scenario
      "A test on the harness").
- [x] 2.3 `claim_and_abandon(store, types, lease, limit)` in `cicada::testing`, exported by `cicada-test` [13].
      Verify: a test for the scenario "A node that claims a turn and dies" passes, and
      `crates/cicada-extras/tests/crash_loop_guard.rs` uses the helper and passes.

## 3. The tests that need the trait move into the core

- [ ] 3.1 The conformance suite becomes a `#[cfg(test)]` module of `cicada`, run for the in-memory store, SQLite
      and, with `CICADA_PG_URL` set, PostgreSQL [11]. Verify: the count per backend equals that of 1.1.
- [ ] 3.2 The tests of the two store crates move into `cicada`, with the helper that builds a version-1 database
      and the upgrade tests: under `crates/cicada/tests/` when they use only public calls, inside the crate when
      they call the trait. All PostgreSQL tests take their databases through one shared limit [11]. Verify: the
      counts equal those of 1.1, and the PostgreSQL tests pass in one run with `CICADA_PG_URL` set.
- [ ] 3.3 The code of `cicada-fuzz` becomes `#[cfg(test)]` modules of `cicada`, with `proptest` as a dev-dependency
      and the regressions files where proptest reads them for a module under `src/` [5, 11]. Verify: the count
      equals that of 1.1; each seed is still replayed (say in the report how you checked, for example by putting
      `claim.epoch + delta` back in a scratch change and seeing `backends_agree` fail on its seed); and
      `PROPTEST_CASES=20000` with the filter of the comparison passes.
- [ ] 3.4 The 11 test files of `cicada-test` that need the trait move into `cicada` with `Faulty`, and so do
      `store_support.rs` and `retry_policy.rs` [7, 11]. Moved tests name the crate as `crate::`. Verify: the counts
      equal those of 1.1.
- [ ] 3.5 The tests that stay outside reach the store through public calls only [11]: `cicada-otel` plants its
      undecodable state through a second system that hosts nothing; `cicada-cli`, `cicada-client`, `cicada-axum`
      and `cicada-otel` open their test stores without naming the trait. Verify: `cargo test --workspace` passes,
      and no file outside `crates/cicada/src` calls a method of the trait.

## 4. One public `Store`, and the trait goes private

- [ ] 4.1 The public type `Store` [4, 8, 10]: `Store::sqlite(path, options)`, `Store::postgres(url, options)`,
      `Store::memory()`, `memory_on(clock)`, `memory_with(clock, retention)`, `close()`, cheap to clone.
      `ActorSystem::builder` takes a `Store`; `ActorSystem::store()` and `TestActors`' `InMemoryStore` go
      (`TestActors::store()` returns the `Store`); `connect_with` goes; `PostgresStoreConfig` becomes
      `PostgresOptions`. Follow in the sample service, the examples, `e2e` and every test. Verify: tests for the
      scenarios "A node on SQLite", "A node on PostgreSQL", "Two systems on one store" and "Closing and opening
      again" pass.
- [ ] 4.2 The trait becomes `pub(crate)` under the name `Backend`, and every type the rule of decision 9 does not
      keep becomes private [1, 9]. `hash_conflict_reason` is public as `cicada::store::hash_conflict_reason`.
      Verify: `cargo build --workspace` has no warning about a private type in a public signature; a doc test
      marked `compile_fail` covers the scenario "A host names a backend type"; an existing test covers "A host
      reads an outcome" (name it in the report).
- [ ] 4.3 Remove the crates `cicada-store-sqlite`, `cicada-store-postgres`, `cicada-store-conformance` and
      `cicada-fuzz` from the workspace and the disk [5, 10]. Doc examples that named `Arc<dyn cicada::store::Store>`
      name `cicada::Store`. Verify: `cargo test --workspace` passes; `cargo test -p cicada` alone runs the
      conformance cases for the in-memory store and SQLite (scenario "The tests of the core alone").

## 5. CI and docs

- [ ] 5.1 `.github/workflows/ci.yml`: a step that builds `cicada` with no feature, with `sqlite`, with `postgres`
      and with `testing`, each alone, and checks with `cargo tree` that the `sqlite` build lists no PostgreSQL
      driver and the `postgres` build no SQLite driver [11]. Verify: the same commands pass locally; they cover the
      scenarios "Only SQLite", "Only PostgreSQL" and "No feature" (these are checked by CI steps, not by a Rust
      test; name the steps in the report).
- [ ] 5.2 `README.md` (the dependency lines with a feature, opening a store, the test section, the crate list),
      `docs/architecture.md` (the crate table and the paragraph on store requirements), `docs/operations.md`
      (opening a store, `PostgresOptions`), `AGENTS.md` (the map and the test commands) and `openspec/config.yaml`
      (the rule that names `cicada-store-conformance`). Verify: `cargo test -p cicada-examples --doc` passes, and
      none of these files names a removed crate.
- [ ] 5.3 `CHANGELOG.md`: the section starts with the upgrade steps of the Migration Plan of `design.md`, then the
      breaking changes of the proposal. Verify: every step of the plan and every **BREAKING** line of the proposal
      is in the section.

## 6. Checks

- [ ] 6.1 Count the tests of each moved suite again and compare with 1.1 (scenario "No test is lost"). Verify: no
      suite has fewer tests; the report has both columns.
- [ ] 6.2 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
      `openspec validate private-store-trait --strict`. Verify: all pass (scenario "A normal test run").
- [ ] 6.3 The PostgreSQL tests with `CICADA_PG_URL` set (scenario "A test run with PostgreSQL"), and the
      end-to-end suite with `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. Both need Docker. Verify: both
      pass; if Docker cannot be started, leave this task open and say so in the report.
