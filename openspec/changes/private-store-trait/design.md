# Design

Described with the owner on 2026-11-11 (change loop, Describe). Every decision below is made by the owner unless it
says otherwise. Reviewed twice. After the first review decisions 6 and 7 changed, decisions 12 and 13 were added
and the measure under Context was counted again. The second review found the shape sound and asked for names, two
missed callers and sharper wording; those are in decisions 6, 9, 10, 11 and 13 now.

## Context

What the code does today (`main` at 93324d6, after release 0.4.0):

- `cicada::store::Store` is a public trait with 33 methods. Six types implement it: `SqliteStore`
  (`crates/cicada-store-sqlite`), `PostgresStore` (`crates/cicada-store-postgres`), `InMemoryStore`
  (`crates/cicada-test`, 900 lines), `Faulty<F>` (a wrapper that injects faults, in
  `crates/cicada-test/tests/common/mod.rs`), `FlakyStore` (a store that can be "down", in
  `crates/cicada-axum/tests/common`) and `FaultStore` (`crates/cicada-fuzz/src/faults.rs`).
- A host opens a store with `SqliteStore::open(path, SqliteOptions)` or `PostgresStore::connect(url, config)` and
  passes `Arc<dyn Store>` to `ActorSystem::builder`. `ActorSystem::store()` hands the trait object back.
- `TestActors` (`crates/cicada-test/src/harness.rs`) runs on `InMemoryStore` and `TokioClock`, on paused tokio time.
  It calls the trait itself: `next_due_at`, `pending`, `list`, `get` and `status`. `TestActors::store()` returns the
  `InMemoryStore`.
- `cicada-fuzz` left its node simulation on `InMemoryStore` for a reason that matters here: sqlx does its I/O on a
  worker thread, and paused tokio time jumps ahead while a query waits, so a run on SQLite is not repeatable.
- Rust has no "friend crates". A trait that is private to `cicada` cannot be implemented or called from another
  crate, and a `tests/` folder is a crate of its own. Only code inside `crates/cicada/src` can reach it.
- Unit tests inside `cicada` cannot use a `TestActors` that lives in `cicada-test`: that crate links the normal build
  of `cicada`, not the test build.

Who calls the trait from outside `crates/cicada/src` today:

| Where | What | Lines |
| --- | --- | --- |
| `cicada-store-conformance` | all of it | 4,555 |
| `cicada-fuzz` | the comparison, the fault layer, the node simulation and their tests | about 3,400 |
| `cicada-test/tests` | 11 of its test files and `common/mod.rs` (`Faulty`, raw `enqueue`, a foreign `claim_due`) | about 5,200 |
| both store crates | their own tests, with `schema::VERSION`, `is_listening` and the upgrade tests of 0.4.0 | – |
| `crates/cicada/tests` | `store_support.rs` and `retry_policy.rs` use `store::support` | – |
| `cicada-extras/tests` | `crash_loop_guard.rs` claims as a node that dies | one call |
| `cicada-axum/tests` | `FlakyStore`, for three HTTP tests of `503` and `500` | – |
| `cicada-otel`, `cicada-cli`, `cicada-client`, `cicada-axum` tests | build an `InMemoryStore` to run a node | – |
| `sample-service` | opens both SQL stores; `system.store().close()` | – |

Chorus (`chorus.rs`, read at its working tree, which pins v0.2.0) calls the trait in production code:
`store.now()`, `store.next_due_at(&system.hosted_types())`, `running_turn`, `pending`, and
`store::support::hash_conflict_reason`. `ActorSystem` has `pending` and `running_turn`, but no `now` and no
`next_due_at`. Its tests build an `InMemoryStore`, read a state with `store.get`, and kill a run with
`store.claim_due`. Karakuri has no cicada dependency.

## Goals / Non-Goals

Goals:

- A new store method or model field is not a breaking change for a host.
- The compiler enforces that nothing outside `cicada` implements or calls a store.
- A host builds only the driver it uses.
- A host's tests on `TestActors` do not change their imports.
- What Chorus does through the trait today, it can do through public calls.

Non-Goals:

- Any change to what a store does for a node: S1–S10 and the schema stay.
- A third backend.
- Choosing the store at run time from a flag or a URL scheme; that is `cicada-app`.

## Decisions

### 1. The trait is private to the crate

`Store` becomes `pub(crate)`; the compiler enforces that nothing outside `cicada` implements or calls it. Everything
that does so today moves into `crates/cicada/src` (the table under Context), or gets a public call instead
(decisions 6, 12 and 13). The test code among it becomes `#[cfg(test)]` modules inside the crate.

Not chosen: public but hidden behind a cargo feature that only this repository's test crates turn on (least churn,
but a host could turn it on); private plus a set of public hooks made for tests (more public surface, not less).

### 2. `InMemoryStore` stays

It stays as the store of `TestActors` and of the node simulation. Running those on SQLite does not work as is
(Context). It moves into `cicada` (decision 1).

Not chosen: SQLite in memory with a virtual clock for all tests (needs a second, synchronous way to drive SQLite).

### 3. Each SQL backend is a cargo feature, none on by default

`cicada` has the features `sqlite` and `postgres`. A host names the one it uses, and builds only that driver.

Not chosen: no features (every host builds both drivers); both on by default.

### 4. One public type, `Store`

The trait gets an internal name. `Store` becomes a public type that a host cannot look into: it holds one backend
and is cheap to clone. A host opens one with `Store::sqlite(path, options)` or `Store::postgres(url, options)`, each
behind its feature, and passes it to `ActorSystem::builder`. After that line host code does not name its backend.

```rust
let store = Store::sqlite("cicada.db", SqliteOptions::default()).await?;
let system = ActorSystem::builder(store.clone()).host(Counter).build();
// …
store.close().await;
```

`ActorSystem::store()` goes. The host holds the `Store` it opened and closes it with `store.close()`. Stopping a
system does not close the store, as today.

Not chosen: three public types as today (`SqliteStore`, `PostgresStore`, `InMemoryStore`) plus a fourth for "any
store".

### 5. `cicada-fuzz` goes away

All of it becomes test code of `cicada`, the parser tests too. The long run becomes `cargo test -p cicada` with a
filter and `PROPTEST_CASES`.

Not chosen: keeping the crate for the 300 lines of parser tests that need only the public API.

### 6. A closed store answers "unavailable"

Changed after the review. First the owner chose unit tests on the error only, on the main session's word that the
end-to-end suite covers a store that is down. That suite runs in the release workflow, not in CI, and `FlakyStore`
backs three HTTP tests, not one.

After `close()`, every store operation of every backend fails as unavailable ("Store is closed"). All three
backends do that already; the conformance suite pins it. The tests of `cicada-axum` for a `503` (health, a send,
`actor_health`) stay over HTTP: they close the store through the public `close()`. `FlakyStore` goes. Its two other
cases, a panic and corrupt data as `500`, become unit tests of `cicada-axum`'s own error mapping and of its wrapper
that catches a panic.

One check cannot be shown with `close()`: that health is `200` again when the store is back. It moves into the core,
as a test of `ActorSystem::health` on the fault wrapper (`Faulty`).

Not chosen: unit tests on the error only (two HTTP tests would be lost); a public "down" switch on the in-memory
store (public API made only for tests).

### 7. The harness code lives in `cicada`, and `cicada-test` exports it

Changed after the review. First the owner chose that `cicada-test` stays as it is; but `TestActors` calls the trait
itself, so it cannot stay outside the core unchanged.

`TestActors`, its builder, the two clocks and the in-memory store live in `cicada::testing`, behind the feature
`testing`. `cicada-test` turns the feature on and exports `TestActors`, `TestActorsBuilder`, `TokioClock`,
`VirtualClock` and `default_start` under the names they have today, next to the snapshot helpers, which stay in
`cicada-test`. A host keeps `cicada-test` as a dev-dependency, and a test on `TestActors` changes no import.
`TestActors::store()` returns the public `Store`, so a test can build a second system on it.

The name `InMemoryStore` goes: there is one public store type (decision 4). A test that built one by hand opens it
with `Store::memory()`, `Store::memory_on(clock)` or `Store::memory_with(clock, retention)`, which take what
`InMemoryStore::default`, `with_clock` and `new` took.

The tests inside the core can use `TestActors`. So the 11 test files of `cicada-test` that need the trait, and
`common/mod.rs` with `Faulty`, move into `cicada` as `#[cfg(test)]` modules nearly unchanged. The test files that
use only the public surface stay in `cicada-test`.

Not chosen: the harness stays in `cicada-test` on public calls only (the 11 files would be rewritten without it).

### 8. PostgreSQL is opened from a URL only

`Store::postgres(url, options)` is the one way. `connect_with(PgConnectOptions, …)` goes, so no type of the sqlx
driver is in the public API and a driver upgrade cannot break a host. A URL carries the host, user, password,
database, TLS mode and application name. An option that a URL cannot carry is added when a host needs it. Chorus and
Karakuri do not use `connect_with`.

Not chosen: keeping both.

### 9. Which store types stay public: the rule

Decided by the main session; it follows from decisions 1 and 4. Corrected after the review.

A type of `cicada::store` stays public when a public function or type of `ActorSystem`, `Turn`, the plugin API,
`TestActors` or the wire model takes, returns or holds it. That keeps: messages, outcomes, pages and cursors, ids,
`ActorRecord` (`CreateActorResult::Exists` holds one), `TurnOutcome` (the plugin API returns it), the views of nodes
and running turns, the results of a cancel, a delete, a deliver-now and a type removal, `StoreError`, `Clock` and
`SystemClock`.

`TypeDef` stays public too: `RegisteredType` holds one, and `ActorSystem::types()` returns that. With it stay
`Contract::to_type_def` and `Contract::of`.

Every other type becomes private: the trait, which is named `Backend` inside the crate, `CommitRequest`,
`CommitResult`, `Claim`, `ActorCreate`, `StateChange`, the results of store-only calls (`CreateResult`,
`EnqueueResult`, `RegisterResult`, `RenewResult`), `schema::VERSION`, `is_listening`, and `store::support`.
`hash_conflict_reason` is the one function of `store::support` that stays public, as `cicada::store::hash_conflict_reason`:
Chorus uses it. The compiler's lint for private types in public signatures shows what is left.

### 10. The layout inside `cicada`, and the features

Decided by the main session; it follows from decisions 1 to 4 and 7.

| Feature | What it adds | Who turns it on |
| --- | --- | --- |
| `sqlite` | `Store::sqlite`, `SqliteOptions` | a host on SQLite; `sample-service`; the examples |
| `postgres` | `Store::postgres`, `PostgresOptions` | a host on PostgreSQL; `sample-service`; the examples |
| `testing` | `Store::memory`, `memory_on`, `memory_with`, and `cicada::testing`: `TokioClock`, `VirtualClock`, `TestActors`, `claim_and_abandon` | `cicada-test` |

- None is on by default. Without a feature the crate builds and can open no store.
- The sqlx driver is an optional dependency, with only the part each feature needs.
- `PostgresStoreConfig` is renamed `PostgresOptions`, to match `SqliteOptions`. Both keep their fields.
- The crates `cicada-store-sqlite`, `cicada-store-postgres`, `cicada-store-conformance` and `cicada-fuzz` are
  removed from the workspace.
- Doc examples that name `Arc<dyn cicada::store::Store>` (`cicada-otel`, `cicada-extras`, the examples) name
  `cicada::Store`.

### 11. Where the tests go

Decided by the main session; it follows from decisions 1, 5, 6 and 7.

- **The conformance suite** becomes a `#[cfg(test)]` module of `cicada`. It runs against the in-memory store and
  SQLite, and against PostgreSQL when `CICADA_PG_URL` is set, as today.
- **The fuzz code** becomes `#[cfg(test)]` modules of `cicada`: the parsers, the comparison, the fault layer and the
  node simulation, with their regressions files. `proptest` becomes a dev-dependency of `cicada`.
- **The tests of the two store crates** move along, with the helper that builds a version-1 database and the
  upgrade tests of 0.4.0. Those that use only `Store::sqlite` or `Store::postgres` and an `ActorSystem` become tests
  in `crates/cicada/tests/`; those that call the trait go inside the crate.
- **The 11 test files of `cicada-test`** that need the trait move into the crate with `Faulty` (decision 7).
  `crates/cicada/tests/store_support.rs` and `retry_policy.rs` move inside the crate too.
- **The tests of `cicada` run with all three features.** `cicada` names itself as a dev-dependency with the three
  features. That turns them on for its unit tests and its tests in `tests/`, while `cargo check -p cicada` stays
  without them (the second reviewer tried it on cargo 1.99). Inside the crate, `cicada::` then names a second copy of
  the crate, so a moved test uses `crate::`.
- **All PostgreSQL tests share one limit.** Today they are separate test binaries that run one after another, and
  the conformance fixture alone is sized for the 100 connections of the CI database. Inside the core they run in one
  binary at once, so they take their databases through one shared limit.
- **The regressions files of proptest move to where it reads them** for a module under `src/`
  (`crates/cicada/proptest-regressions/`), and the tasks check that each seed is still replayed.
- **`cicada-axum`** tests a store that is down through `close()` (decision 6). Its one direct `store.list` becomes
  `system.list_actors`.
- **`cicada-otel`** plants a state that does not decode through a second system that hosts nothing, as a node that
  does not host the type would.
- **`cicada-extras`** (`crash_loop_guard.rs`) uses the helper of decision 13.
- **CI and the finishing checks** build each feature alone, because `cargo test --workspace` turns every feature on:
  `cargo check -p cicada` with no feature, with `sqlite`, with `postgres` and with `testing`.
- **No test is lost.** The tasks count the tests of every moved suite before and after.

### 12. `ActorSystem::now()` and `ActorSystem::next_due_at()`

Added after the review. Chorus reads both from the store today, and the harness needs `next_due_at` as well.

- `now()` answers the store's time.
- `next_due_at()` answers the earliest time at which a message of a type this system hosts is due, or nothing when
  none is pending.

With `pending` and `running_turn`, which exist, a host can then do through the system everything Chorus does
through the trait in production code.

### 13. One test helper for a node that claims a turn and dies

Added after the review. Three tests claim through the trait as a node that is never heard of again: one of
`cicada-test`, `crash_loop_guard.rs` of `cicada-extras`, and Chorus's test of a run that is killed during an
attempt. The last two cannot move into the core.

`cicada::testing` gets one function for it, behind `testing` and exported by `cicada-test`:
`claim_and_abandon(store, types, lease, limit)`. It claims what is due as a node of its own and returns one
`AbandonedClaim` per claim, with the address and the message id. It commits nothing, so the lease runs out and the
message is claimed again, as after a crash. It works on every backend.

Not chosen: no helper (Chorus and `cicada-extras` lose those tests).

## Risks / Trade-offs

- **The core crate grows**: about 6,000 lines of stores, the harness, and about 13,000 lines of test code, and its
  test build takes longer. → Accepted with decision 1; the public API gets smaller, which is the aim.
- **A feature combination that nobody builds breaks unseen.** → CI builds each feature alone (decision 11).
- **A host that names no feature can open no store.** → The README and the docs of `Store` say which feature to
  name; the dependency line in the README shows it.
- **Test support is compiled into the core when `testing` is on.** → Only `cicada-test` turns it on, and a host has
  that as a dev-dependency, so a host's release build does not hold it.
- **A host needs a connect option that a URL cannot carry.** → Added when it comes up (decision 8).
- **Moving this much code can lose a test on the way.** → The tasks count the tests before and after, per suite.
- **Tests now lean on a closed store answering "unavailable".** → All three backends do so already, and a
  conformance case pins it.

## Migration Plan

For a host (Chorus, Karakuri):

1. `Cargo.toml`: drop `cicada-store-sqlite` or `cicada-store-postgres`; name the feature on `cicada`
   (`features = ["sqlite"]`). `cicada-test` stays a dev-dependency.
2. Open the store with `Store::sqlite(path, options)` or `Store::postgres(url, options)`, and pass it to
   `ActorSystem::builder`. Replace `Arc<dyn Store>` with `Store`, and `PostgresStoreConfig` with `PostgresOptions`.
3. Replace calls on the store with calls on the system: `store.now()` with `system.now()`, `store.next_due_at(…)`
   with `system.next_due_at()`, `store.running_turn(…)` and `store.pending(…)` with the methods of the same name.
   `hash_conflict_reason` is now `cicada::store::hash_conflict_reason`.
4. Replace `system.store().close()` with `store.close()` on the store the host opened.
5. In tests: `TestActors` is unchanged. Replace `InMemoryStore::default()`, `with_clock(…)` and `new(…)` with
   `Store::memory()`, `Store::memory_on(…)` and `Store::memory_with(…)`; read a state with `system.get_actor(…)`
   instead of `store.get(…)`; replace a `store.claim_due(…)` that plays a crashed node with `claim_and_abandon`.

No database changes. The version is a breaking one: 0.5.0 after 0.4.0.
