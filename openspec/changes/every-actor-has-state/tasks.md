# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`.

## 1. `NoState` replaces `Stateless`

- [x] 1.1 `cicada::message`: add `NoState` with `deny_unknown_fields` [4]; remove `Stateless` and `ActorState`; give
      `Actor::State` the bound `Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static`; export `NoState`
      from the crate root and the prelude. The doc comment of `Actor::State` says that a state is never `null` and
      points to `NoState`. Verify: unit tests that `NoState {}` encodes to `{}` and that `{"oops": 1}` does not
      decode as `NoState`.
- [x] 1.2 Remove `create_stateless` from `Turn`, `ActorSystem`, `ActorClient` and `TestActors`. Replace every
      `Stateless` and `create_stateless` in the tests, the examples, the sample service, `cicada-fuzz` and `e2e`
      with `NoState` and `create(&r, &NoState {})`. Delete the three snapshots of formerly stateless types in
      `sample-service/tests/contracts` and let the test write them again [9]. Verify: `cargo test --workspace`, and
      no match for `Stateless` or `create_stateless` in the `.rs` files outside `target/`.
- [x] 1.3 A state is never `null` [2]: `Turn::create` returns `TurnError::Invalid`, `ActorSystem::create_actor`
      returns `InvalidState`, and a turn that ends with a `null` state ends its message as `failed`. Remove the
      refusal "is stateless and takes no state" from `create_actor` and `check_typed_effects`. Verify: tests on
      virtual time with `TestActors` for the scenarios "A turn creates with a null state" and "A turn sets a null
      state" (the message is `failed` after one attempt and the draft is still there), and a test that a create with
      a `null` state from outside a turn is refused.

## 2. A state is never absent: store, contract, wire and command line

- [x] 2.1 Store model and trait [8]: `Claim.state`, `ActorRecord.state` and `ActorCreate.state` are a `Value`,
      `StateChange::NewState` holds a `Value`, `Store::create` takes a `Value`. Follow in `InMemoryStore`,
      `SqliteStore`, `PostgresStore`, `cicada-store-conformance`, `cicada-fuzz` and the runtime. Verify: the
      conformance suite passes for the in-memory and the SQLite store, `cargo test -p cicada-fuzz` passes, and
      `CICADA_PG_URL=postgres://cicada:cicada@localhost:55432/cicada cargo test -p cicada-store-postgres` passes.
- [x] 2.2 Migration 2 in both stores [1, 5]: states that are SQL `NULL` or JSON `null` become `{}`; registrations
      whose contract has no state schema are removed; `actors.state` is `NOT NULL` without a default (SQLite rebuilds
      the table and creates `actors_due` and `actors_running` again). Update the module docs of both `schema.rs`
      files: schema version 2, and this migration is not additive. Verify: the tests of 2.3.
- [x] 2.3 Tests of the upgrade [10]: a helper under `#[cfg(test)]` in each SQL store crate applies the migrations up
      to a given version. With a version-1 database built by it, test in SQLite and in PostgreSQL: "Instances with
      and without a state", "The store refuses a missing state" and "A type with state keeps its registration"; in
      SQLite also "The indexes of SQLite survive". With a real `ActorSystem` on a migrated SQLite file, test "A
      stateless type is upgraded in place" (the node starts, the type is registered again, the actor has `{}`, the
      pending message is handled) and "A type that no node hosts any more". Verify: these tests pass, the
      PostgreSQL ones with `CICADA_PG_URL` set.
- [x] 2.4 `Contract` [1, 8]: the state schema is a `Value` in `new`, `state_schema()` and the JSON form;
      `from_json` refuses a contract whose `state` is absent or `null`; `describe_changes` in
      `cicada-test/src/snapshots.rs` follows. Verify: unit tests for both refusals and for the JSON form of a
      `NoState` type.
- [x] 2.5 Wire model, HTTP and client [3, 8]: `CreateActorRequest.state` is required; `ActorView.state`,
      `TypeView.state` and `ActorSnapshot.state` are a `Value`; `PUT /actors/{type}/{key}` answers `400` for an empty
      body and for a body without `state`; `ActorClient::create_actor` takes a `Value`. Regenerate
      `docs/openapi.json`. Verify: tests in `cicada-axum` for the scenarios "A create over HTTP with a state",
      "without a body", "without the state field", "A null state over HTTP", "A state with a field for a type that
      keeps no data", "An actor of a type that keeps no data" and "A type that keeps no data" (`GET /types/{name}`);
      the OpenAPI test passes.
- [x] 2.6 `cicada-cli` [6]: `--state` is required in `cicada create actor`; the TUI drops the "Stateful" fact and the
      types table drops its state column; the code for a missing state goes. Verify: tests in `tests/cli.rs` for
      "The command line without a state" (an error that names `--state`, and no request reaches the node) and "The
      command line with a state"; the `TestBackend` tests pass.
- [x] 2.7 `e2e` [8]: every create in the suite sends a `state`. Verify: `cargo test -p cicada-e2e --no-run` builds;
      the suite itself runs in 4.2.

## 3. Docs

- [ ] 3.1 `README.md` (the turn API, the HTTP table, the test harness, the examples table), `docs/architecture.md`
      (state is never absent; the schema) and `docs/operations.md` (schema version 2; the Upgrading section says
      that a release may need every old node stopped first, and that this one does). Verify:
      `cargo test -p cicada-examples --doc` passes, and no match for `Stateless`, `create_stateless` or "stateless"
      is left in these three files except where they describe the upgrade.
- [ ] 3.2 `CHANGELOG.md`: the section of this version starts with the upgrade note, taken from the Migration Plan
      of `design.md` (the code changes; delete the contract snapshots of stateless types; a state type that encodes
      to `null` is changed to a struct and its type reset; stop every old node; start the hosts of the formerly
      stateless types first), then the breaking changes of the proposal. Verify: every step of the Migration Plan
      and every **BREAKING** line of the proposal is in the section.

## 4. Checks

- [ ] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
      `openspec validate every-actor-has-state --strict`. Verify: all pass.
- [ ] 4.2 The PostgreSQL tests with `CICADA_PG_URL` set, and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. Both need Docker. Verify: both pass; if Docker cannot
      be started, leave this task open and say so in the report.
