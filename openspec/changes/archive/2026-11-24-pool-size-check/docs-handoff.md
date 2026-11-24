# Docs handoff for pool-size-check

Version: 0.21.0 (joins the unreleased batch; no new version). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

A node that hosts at least one type and starts on PostgreSQL with a pool smaller than its concurrency plus 4 logs one
`tracing` warning at WARN level, then starts as before. No public API, wire format, schema, guarantee, command-line
or skill route changes; `cicada-app` has no code change (its `serve` builds a node, which checks). The text, for a
pool of 20 and a concurrency of 64:

> The PostgreSQL pool has 20 connections, fewer than the 68 this node needs (concurrency 64 + 4): lease renewals can
> wait for a connection and turns can be lost. Raise the pool (PostgresOptions::max_pool_size, or --store-pool-size /
> CICADA_STORE_POOL_SIZE in cicada-app) or lower the concurrency (NodeConfig::concurrency, or --concurrency /
> CICADA_CONCURRENCY in cicada-app).

When it fires: once, in `ActorSystem::start` (not `initialize`), only on PostgreSQL, only when the node hosts at least
one type, and only when `max_pool_size < concurrency + 4`. A pool of exactly `concurrency + 4` does not warn. SQLite
and the in-memory store have no pool and never warn; a send-only node never warns. Nodes, or a local `Worker` on a
node, that share one cloned `Store` share one pool: each node checks only its own concurrency, and the sum is not
checked. HTTP workers' load on a server's pool is not checked either.

Proved by `a_pool_below_the_concurrency_plus_4_warns_with_both_numbers_and_the_settings` (unit test of
`pool_warning` in `crates/cicada/src/runtime/system.rs`) and by
`a_node_warns_at_start_when_its_pool_is_smaller_than_its_concurrency_plus_4` in
`crates/cicada/src/tests/postgres/runtime.rs` (pool 6, concurrency 4: one warning naming 6, 4 and 8, node started;
pool 8 and a send-only node on pool 1: none). The existing `pool_size` tests of `cicada-app` show a given pool is
kept and the default is at least the concurrency plus 4.

## 1. The store reference: the pool row

**Where.** `docs/reference/stores.md`, the PostgreSQL options table, the `max_pool_size` row.
**Text.** Keep "At least `concurrency + 4`; see Sizing." and add: a node that hosts types warns at start (one
`tracing` warning naming both numbers and the settings) when the pool is smaller; it still starts.
**Caveats.** Each node checks only its own concurrency; nodes or a local `Worker` sharing one `Store` share its pool,
and their sum is not checked.

## 2. Operations: the sizing table

**Where.** `docs/operations.md`, `Sizing`, the `PostgreSQL pool` row.
**Text.** In the signal column, next to "Renewal delays from connection starvation": the node's start-time warning
"The PostgreSQL pool has N connections, fewer than the M this node needs …" when the pool is below concurrency + 4.
**Caveats.** A warning, not a refusal; only at start; not summed over nodes sharing one store; HTTP workers' load
on the server's pool is not covered.

## 3. App guide: the pool flag

**Where.** `docs/guide/app.md`, the settings table, the `--store-pool-size` row.
**Text.** A given pool is used as given, also when it is smaller than the concurrency plus 4; the node then warns at
start. The default (the larger of 20 and the concurrency plus 4) never warns.
**Caveats.** `worker` over HTTP opens no store and refuses `--store-pool-size`, as before.

## Blocking

None.

## Status

Ran `cargo test -p cicada-examples --test docs` and `--test skill` (see the build report). All text above is
proposed; no protected prose was edited.

(From the code review: the warning comes once per start; a node stopped and started again warns again.)
