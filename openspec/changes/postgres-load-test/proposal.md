# Proposal

## Why

The owner asked how far Cicada scales on PostgreSQL: how many nodes and workers can share one store. Nobody knows.
The repo has no benchmark or load test, and throughput is not a goal (docs/architecture.md). The sizing table in
docs/operations.md gives starting points, not limits. Operators need a measured answer, even a rough one.

Reading the code points to these likely limits. None of them is measured:

- **Connections.** Each `serve` node or `worker --store` holds its pool (default 20) plus up to 2 more connections.
  PostgreSQL's default `max_connections` of 100 fits about 4 nodes.
- **Claims scan running turns.** `claim_due` does not clear `next_due_at`, so a leased actor stays in the
  `actors_due` index. Every claim from every node scans past all running turns in the store.
- **NOTIFY at commit.** Almost every commit sends a `NOTIFY`. In PostgreSQL, commits that notify take a lock for the
  whole database while they commit, so they may commit one at a time. This comes from PostgreSQL itself, not this
  repo, and may differ between versions.
- **Wake-ups reach every node.** Each notification wakes every node that hosts its type, and each one runs a claim.
- **Pools smaller than concurrency.** Renewals share the pool with commits. A renewal that waits too long loses the
  turn, which counts toward the claim limit.

## What Changes

- An opt-in load test, run by hand against a PostgreSQL URL. It is not part of `cargo test` or the gate.
- The workload is one simple actor type spread over many actors, so the one-turn-per-actor rule is not the limit.
  The handler has a configurable delay: none (fast turns) or an async sleep (slow turns that hold no connection).
- Load rises in steps. Each step holds a fixed send rate for a fixed time, and the next step raises it, until the
  system can no longer keep up (the backlog grows during a step) or errors appear. The run repeats this for 1, 2, 4
  and 8 nodes.
- Each step records: the rate sent and the rate completed, turns per second, latency from send to commit (p50, p95,
  p99), the backlog at the end, lost claims and errors. From PostgreSQL: connections in use and the most common wait
  events, sampled from `pg_stat_activity`. The wait events show which of the limits above is reached.
- Each run writes one table with its setup: versions, nodes, concurrency, pool size, handler delay and machine.
- The results of a first run go to the docs session in the docs handoff, for the sizing table in docs/operations.md.

To decide in the design:

- Where the nodes run: several `ActorSystem`s in one process, each with its own store and pool (simplest; they
  share one CPU), or separate containers like the e2e suite (closer to production; slower to set up and run).
- Where the tool lives: its own crate, a binary in `e2e/`, or an ignored test.
- How load rises: by send rate (finds where the system falls behind) or by the number of senders that wait for
  each answer (simpler, but hides the point where it falls behind).
- Which handler delays and step sizes to use, and how long one step lasts.
- Whether pool size and concurrency are varied in the run, to answer the pool question with data.
- Which PostgreSQL version to measure: 17.5 (docker-compose.yml) or 18 (e2e).
- Whether a change that adds only a tool needs a release.

## Out of scope

Fixing what the test finds: each limit it confirms becomes its own proposal. Any change to runtime or store
behaviour, schema, wire format, public API or guarantees. Pass/fail thresholds and running in the gate. HTTP workers
(`worker --server`), SQLite, several hosts and PostgreSQL tuning; a later change can add them.

## Order

After `node-view-rustdoc` (in build), before `pool-size-check`.

## Impact

A new opt-in tool and its workspace entry. No change to the behaviour of any published crate, so no new ADR. The docs
handoff carries a how-to for running it and the first results for docs/operations.md; the docs session writes both.
