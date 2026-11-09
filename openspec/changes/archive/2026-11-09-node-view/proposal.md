# Proposal

Status: built on 2026-11-09.

## Why

Operators want to see which nodes exist, what they host and whether they are alive. Nodes are not recorded anywhere
today, so a node that hosts nothing, or a type no live node hosts, cannot be seen. A leader lease such as Oban's cannot
serve: it holds one row. Quartz shows that per-node check-ins in the shared database are enough.

## What Changes

- **N1.** A new `nodes` table with one row per node, send-only nodes included: id, hosted types, started and last seen.
  Every node heartbeats its row using the store clock, so node clocks are never compared.
- **N2.** `GET /nodes` and `GET /nodes/{node}` serve the status (alive, suspect or dead, derived from the age of the
  last heartbeat) and the node's running turns.
- Defaults (a judgement call, to confirm in the design): heartbeat 10 s, suspect after 30 s, dead after 2 min, row
  removed after a day. Any node removes old rows during its heartbeat, so no separate cleanup job exists.

The table is for display only. Leases stay the only authority over who runs what, so a wrong node status cannot cause
overlapping turns.

## Out of scope

- Remote control of nodes (stop, drain).
- Leader election, or any coordination through this table.
- Metrics per node.

## Impact

- New `Store` methods in all three stores and new conformance cases. The schema changes in place at version 1
  (ADR 0008).
- `ActorSystem` (the heartbeat task), `cicada-axum`, `docs/operations.md`.
- Guarantees G1-G6 and S1-S10 do not change.
