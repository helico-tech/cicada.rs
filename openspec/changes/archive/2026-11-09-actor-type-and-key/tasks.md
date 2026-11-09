# Tasks

## 1. Core

- [x] 1.1 `TypeName`, `ActorAddress` as type plus key, `RefBinding` on field names. Verify: unit tests in `address`.
- [x] 1.2 Contract, resolver, runtime, plugin info and wire model use type names. Verify: `cargo test -p cicada --lib`.
- [x] 1.3 ADR 0013. Verify: linked from the README.

## 2. Stores

- [x] 2.1 Schema columns and SQL in both stores, `InMemoryStore`. Verify: `cicada-store-conformance` passes on all three.

## 3. HTTP

- [x] 3.1 Routes `/actors/{type}/{key}` and `/messages`; client paths. Verify: `cargo test -p cicada-axum -p cicada-client`.

## 4. The rest

- [x] 4.1 `cicada-extras`, `cicada-otel`, `examples`, `sample-service`, `e2e` and their tests, with recorded contracts and
      state samples. Verify: `cargo test --workspace`; end-to-end suite with `CICADA_E2E=1`.
- [x] 4.2 README, `docs/architecture.md`, `docs/operations.md`, `CHANGELOG.md`. Verify: the doctests of `cicada-examples`.

## 5. Finish

- [x] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
      `openspec validate actor-type-and-key --strict`.
