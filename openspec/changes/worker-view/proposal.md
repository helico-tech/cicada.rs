# Proposal

## Why

An HTTP worker leaves no trace an operator can read while it is idle. `GET /nodes` never lists it (ADR 0048, the
`node-view` spec: "a remote Worker SHALL add no node row"); `GET /turns?node=<worker id>` shows it only while it holds a
lease. So an operator cannot tell a worker that is down from one that polls and finds no work. That matters most for
placement (ADR 0052): work pinned to a worker id stays pending while that worker is away, with no sign of why. The
inspector and the tray have the same blind spot. The owner picked a store-backed list on 2026-11-28 (W1).

## What Changes

- Each `POST /leases` already names the worker (`owner`) and its types with their contract hashes. The core's lease
  path records them in a new store table: worker id, hosted type names, first seen and last seen, in store time. The
  write is throttled per worker (at most one every few seconds), so polling does not mean a write per request.
- A new route `GET /workers` lists the recorded workers by id, each with its hosted types, `firstSeen`, `lastSeen`
  and a status `alive`, `suspect` or `dead` from the age of `lastSeen`, as for nodes. `cicada-client` gains
  `workers()`.
- The list is for display only. Nothing reads it to decide who runs what: leases, sweeps, node liveness and placement
  stay as they are. Worker rows live apart from node rows.
- An appended schema migration adds the table. An ADR records the rule, amending ADR 0048's "no row" for workers.

To decide in the design: the throttle interval and where it is kept (SQL condition or in memory); how long a dead
worker's row stays and who removes it (a worker never says it stopped); whether one worker id seen with different
type sets keeps the latest set; and whether `GET /workers/{id}` with its running turns is worth adding now.

## Out of scope

- New fields on the lease request (host name, version, label, concurrency): a wire change, a later change if wanted.
- A `cicada workers` command, its terminal UI table, and a Workers view in the inspector or the tray: follow-up changes.
- Direct-store workers and nodes: they keep their node rows.
- Any use of the list for liveness, fencing or placement decisions.

## Order

After `inspector-outcomes-mockup`. No following change.

## Impact

Crates: cicada, cicada-axum, cicada-client.

`crates/cicada/src/schema.rs` (an appended migration), the store backends and the conformance suite
(`crates/cicada/src/tests/conformance`), the core lease path, `crates/cicada/src/api` (the view),
`crates/cicada-axum` (the route), `crates/cicada-client` (`workers()`). Regenerated `docs/openapi.json`. A new ADR and
a `node-view` (or new `worker-view`) spec delta. Docs handoff: the HTTP guide and operations text about seeing
workers, `docs/architecture.md`'s worker section and the "no node row" lines in the app guide and operations.
