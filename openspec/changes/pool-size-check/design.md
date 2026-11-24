# Design

## Context

The node takes its concurrency from `NodeConfig::concurrency`; the store's pool is `PostgresOptions::max_pool_size`,
fixed when `Store::postgres` opens it, before the node exists. `ActorSystem::builder(store)` gets the `Store`, which
implements the private `Backend` trait itself and delegates to its backend (ADR 0018). `cicada-app` resolves its pool in
`Settings::pool_size`: the given one, else `max(20, concurrency + 4)`, and then builds an `ActorSystem` on that store.

## Decisions

1. **Warn, do not refuse.** `ActorSystem::start` logs one `tracing::warn!` and starts as before. A warning breaks no
   setup that works today and stays a patch. The runtime has no start event for observers (`TurnObserver` sees turns
   only), and its other start-time messages are `tracing` lines, so the warning is one too.
   Not taken: refusing to start (stops working setups, needs a minor version); a new observer start event (new public
   API for one line).
2. **The rule is exactly `pool < concurrency + 4`.** It is the documented rule (docs/reference/stores.md,
   docs/operations.md) and the one `cicada-app` sizes by, so the three agree. No load test measures a smaller margin
   (`postgres-load-test` was dropped). Computed without overflow (`concurrency.saturating_add(4)` against the pool as
   `usize`). Not taken: a smaller margin (no evidence for it).
3. **The node learns the pool through the private trait.** `Backend` gets `fn pool_size(&self) -> Option<u32>` with
   a default `None`; the PostgreSQL backend returns its `max_pool_size`; `Store` delegates, as it does for `wake_ups`.
   SQLite and the in-memory store keep the default. No public surface. Not taken: a public `Store::pool_size()`
   (`cicada-app` does not need it, see 5); a field in `NodeConfig` (a second place to state the pool, which can lie).
4. **When and what.** The check runs in `start`, once per node, only when the node hosts at least one type (a node
   without types runs no turns, so its concurrency uses no connections), not in `initialize`. The text comes from a
   pure function `pool_warning(pool: Option<u32>, concurrency: usize) -> Option<String>` in the runtime, so the rule
   and the words are unit-tested without a subscriber. The text names both numbers and both settings, e.g.:
   `The PostgreSQL pool has 20 connections, fewer than the 68 this node needs (concurrency 64 + 4): lease renewals can
   wait for a connection and turns can be lost. Raise the pool (PostgresOptions::max_pool_size, or --store-pool-size /
   CICADA_STORE_POOL_SIZE in cicada-app) or lower the concurrency.`
   Not taken: a library-only text (an app operator would read a setting they cannot set); a second warning in
   `cicada-app` (two lines for one problem).
5. **`cicada-app` needs no code.** `serve` builds an `ActorSystem` on its store, so an explicit `--store-pool-size`
   or `CICADA_STORE_POOL_SIZE` below concurrency + 4 gets the core warning. The default is `max(20, concurrency + 4)`
   and never warns. `worker` over HTTP opens no store and refuses `--store-pool-size` already. Only the app spec
   changes: it says a given pool is kept; the warning belongs to store-backends. Not taken: an app-side check on `Settings` (duplicates the rule and the warning).
6. **Tests.** Unit tests of `pool_warning`: `None` pool never warns; `concurrency + 4` exactly does not; one less
   does, and the text holds both numbers and both setting names; a huge concurrency does not overflow. A PostgreSQL
   runtime test in `crates/cicada/src/tests/postgres/` (skipped without `CICADA_PG_URL`, real time like its
   neighbours, since the pool exists only there) starts a node with pool 6 and concurrency 4 hosting one type and
   captures the warning with a thread-local `tracing-subscriber` test writer (`tracing::subscriber::set_default`; a
   workspace dependency, added to `cicada`'s dev-dependencies). That subscriber sees only its own thread, so the test
   runs on a single-thread runtime (`#[tokio::test]`, not `multi_thread`) and `start` logs the warning itself, before
   it spawns anything. A node with no types, and one at pool 8, logs none. A virtual-time `TestActors` test is not
   needed: the in-memory store has no pool, which the unit test covers. `cicada-app` needs no new test: the existing
   `pool_size` tests already show a given pool is kept (5) and the default follows the rule (44); the warning is the
   core's.
7. **Version.** A patch; it joins the unreleased batch (0.21.0 once `container-limits-and-kinds` has landed) in
   `CHANGELOG.md`.

## Out of scope

As in the proposal: changing the default pool or concurrency; sizing the pool from the concurrency in embedded nodes;
the load of HTTP workers on a server's pool; SQLite. Nodes, or a local `Worker` on a node (`ActorSystem` is a
`TurnSource`), that share one cloned `Store` share one pool: each node checks only its own concurrency, and the sum
is not checked. The docs handoff does not promise more.

## Docs

Breaks nothing: no public API, doctest, link, guarantee row or skill route changes. Handoff items:
docs/reference/stores.md (the `max_pool_size` row: a node warns at start below `concurrency + 4`), the sizing table
in docs/operations.md (the PostgreSQL pool row), and docs/guide/app.md (the `--store-pool-size` row: an explicit
smaller pool warns).
