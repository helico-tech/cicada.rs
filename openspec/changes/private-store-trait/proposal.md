# Proposal

## Why

The `Store` trait is public, with more than 30 methods and the model types of a commit. It has two real backends, both
in this repository (PostgreSQL and SQLite), and an in-memory one for tests. Nobody outside writes a backend, yet every
new store method or field is a breaking change of the public API. A host depends on a store crate of its own, opens
the store there and passes `Arc<dyn Store>` to `ActorSystem::builder`. ADR 0018 decides that the store trait is not an
extension point.

## What Changes

- **BREAKING** The `Store` trait becomes private to `cicada`, and so do the types only a backend needs
  (`CommitRequest`, `Claim`, `ActorCreate`, the results of store-only calls). The compiler enforces it.
- **BREAKING** `Store` becomes a public type that a host cannot look into. A host opens one with
  `Store::sqlite(path, options)` or `Store::postgres(url, options)` and passes it to `ActorSystem::builder`.
  `ActorSystem::store()` is removed; the host closes the store it opened, and a closed store answers "unavailable".
- **BREAKING** The two SQL stores move into the `cicada` crate (layer 0 of ADR 0018), each behind a cargo feature,
  `sqlite` and `postgres`. No feature is on by default. The crates `cicada-store-sqlite` and `cicada-store-postgres`
  are removed.
- **BREAKING** PostgreSQL is opened from a URL only; `connect_with` and its sqlx options go.
  `PostgresStoreConfig` is renamed `PostgresOptions`.
- `ActorSystem` gets `now()` and `next_due_at()`, so that a host reads through the system what it read from the
  store.
- `InMemoryStore` stays as the store of `TestActors`. It moves into `cicada` with the harness code and the test
  clocks, behind the feature `testing`. `cicada-test` stays and exports `TestActors` and the clocks under the names
  they have today, so a host's tests on `TestActors` do not change. The name `InMemoryStore` goes: a test that built
  one opens it with `Store::memory…`. A new test helper plays a node that claims a turn and dies.
- The types a host reads stay public: messages, outcomes, pages, ids, and the views of nodes and turns.
- `cicada-store-conformance` and `cicada-fuzz` are removed as crates. Their code becomes test code inside `cicada`,
  because nothing outside the crate can call the trait.

The simplest version: no plug-in point is kept "just in case".

## Capabilities

### New Capabilities

- `store-backends`: which backends exist and how a host opens one.

### Modified Capabilities

None. `sqlite-store` keeps its requirements.

## Out of scope

- Any change to what a store does: S1–S10 and the schema stay as they are.
- A third backend.
- Reading the store from command line flags; that is `cicada-app`.

## Order

Step 2 of ADR 0018. After `every-actor-has-state`, before `one-name-keys`.

## Impact

- `crates/cicada` (it gains the stores, the harness code and their tests), both store crates,
  `cicada-store-conformance` and `cicada-fuzz` (removed), `cicada-test`, `cicada-extras`, `cicada-otel`, the tests of
  `cicada-axum`, `cicada-cli` and `cicada-client`, the examples, the sample service, `e2e`, the workspace
  `Cargo.toml`, `.github/workflows/ci.yml` (a build per feature), `openspec/config.yaml` (it names
  `cicada-store-conformance`), `AGENTS.md`, `README.md`, `docs/architecture.md` (the crate table and the paragraph on
  store requirements), `docs/operations.md` and `CHANGELOG.md`.
- Covered by ADR 0018; no further ADR.
- Chorus opens `SqliteStore` from `cicada-store-sqlite`, holds an `Arc<dyn Store>`, calls `now`, `next_due_at`,
  `running_turn` and `pending` on it, and uses `InMemoryStore` and a foreign claim in its tests. Chorus must follow;
  `design.md` has the steps. Karakuri has no cicada dependency.
