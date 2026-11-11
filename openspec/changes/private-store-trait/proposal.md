# Proposal

## Why

The `Store` trait is public, with more than 30 methods and the model types of a commit. It has two real backends, both
in this repository (PostgreSQL and SQLite), and an in-memory one for tests. Nobody outside writes a backend, yet every
new store method or field is a breaking change of the public API. A host depends on a store crate of its own, opens
the store there and passes `Arc<dyn Store>` to `ActorSystem::builder`. ADR 0018 decides that the store trait is not an
extension point.

## What Changes

- **BREAKING** The `Store` trait and the types only a backend needs (`CommitRequest`, `Claim`, `ActorCreate`, the
  result enums of store calls) leave the public API of `cicada`.
- The two SQL stores move into the `cicada` crate (layer 0 of ADR 0018). The crates `cicada-store-sqlite` and
  `cicada-store-postgres` are removed.
- **BREAKING** A host opens a store through `cicada`, from a file path or a PostgreSQL URL, and passes the result to
  `ActorSystem::builder`. `ActorSystem::store()` is removed.
- The types a host reads stay public: messages, outcomes, pages, ids, and the views of nodes and turns.
- `cicada-store-conformance` becomes the store test suite of this repository. It is no longer a crate that others run.

The simplest version: no plug-in point is kept "just in case".

To decide in the design:

- Whether `InMemoryStore` stays, or tests run on SQLite in memory with a virtual clock (ADR 0018, Left to the
  changes).
- Whether each SQL backend is a cargo feature, so that a SQLite host does not build the PostgreSQL driver.

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

- `crates/cicada`, both store crates (removed), `cicada-store-conformance`, `cicada-test`, `cicada-extras`, the tests
  of `cicada-axum`, the examples, the sample service, `e2e`, `README.md`, `docs/architecture.md` (the crate table),
  `docs/operations.md` and `CHANGELOG.md`.
- Covered by ADR 0018; no further ADR.
- Chorus opens `SqliteStore` from `cicada-store-sqlite`, holds an `Arc<dyn Store>` and uses `InMemoryStore` in its
  tests. Chorus and Karakuri must follow.
