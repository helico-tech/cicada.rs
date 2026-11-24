# Proposal

## Why

A node on PostgreSQL needs a pool of at least its concurrency plus 4 (docs/reference/stores.md, docs/operations.md).
Nothing checks this. An embedded node takes its concurrency from `NodeConfig::concurrency` and its pool from
`PostgresOptions::max_pool_size` (default 20), and the two are never compared: a node with concurrency 64 runs on a
pool of 20 without a word. In `cicada-app` the pool follows the concurrency only when `--store-pool-size` is absent;
a smaller pool given by flag or variable is taken as it is.

A pool that is too small can starve lease renewals under load. Renewals wait in the same queue as claims and commits.
A turn whose renewal is not confirmed before its lease deadline is cancelled and lost, and lost turns count toward
the claim limit, which then fails the message for good. So the cost is failed messages, not only slower ones.
Found while answering how far Cicada scales on PostgreSQL (2026-11-17).

## What Changes

- A node that starts on PostgreSQL with a pool smaller than its concurrency plus 4 tells the operator, naming both
  numbers and the setting to change.
- The same applies to an explicit `--store-pool-size` in `cicada-app`.

To decide in the design:

- Warn at start or refuse to start. A warning breaks nobody, but is easy to miss. Refusing catches every case, but
  stops setups that work today, such as slow handlers with few commits.
- Whether the rule is exactly concurrency plus 4, as `cicada-app` uses, or a smaller margin. The pool series of
  `postgres-load-test` can show which margin matters.
- How the node learns the pool size. The store is built before the node, and the backend trait is private
  (ADR 0018), so the store could report its pool size to the runtime; SQLite and the in-memory store have none.

## Out of scope

Changing the default pool size or concurrency. Sizing the pool from the node's concurrency in embedded nodes: the store
and its pool exist before the node starts. The load of HTTP workers on a server's pool, which the server cannot know
in advance. SQLite.

## Order

After `postgres-load-test`, before `container-consistency`.

## Impact

Node start in `crates/cicada` and pool resolution in `crates/cicada-app`, with their tests. No wire format, schema or
guarantee change. Refusing to start would change the behaviour of existing setups and needs a minor version; a
warning is a patch. The docs handoff covers the store reference and the sizing table in docs/operations.md.
