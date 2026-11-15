# Proposal

## Why

The node table (`GET /nodes`, `cicada get nodes`, the TUI) shows each node's id, hosted types, start time, last
heartbeat and health. An operator who sees a suspect or dead node cannot tell which machine or container it was, or
which version it ran: the id is random unless the host sets `CICADA_NODE_ID`, and the row holds nothing else. The
next two changes need this too: `pinned-actors` pins an actor to a node by its id, so that id must be stable and
easy to find, and `node-down-alert` tells a watcher which node went down.

## What Changes

- **Node details in the row.** A node records a few facts about itself when it starts: the host name, the Cicada
  version, and an optional free-text label from its `NodeConfig`. They show in `NodeInfo`, `GET /nodes`,
  `GET /nodes/{node}`, `cicada get/describe node` and the TUI.
- **A stable id is easy to choose.** Guidance, and maybe a helper, for deriving a node id from the host name, so that
  a restarted node keeps its id. The random id stays the default.
- The schema gets an appended migration for the new columns.

To decide in the design:

- The exact fields. Host name and version are the minimum. Whether the process id, the address the HTTP API listens
  on, or free key-value labels are also worth it now.
- Whether the id may default to the host name in `cicada-app` (stable, but two nodes on one host collide), or stays
  random with the host name only shown.
- What a node that starts with the id of a row that is still alive does: today two nodes may share an id.
- Whether a worker (no node row today) reports its details anywhere.

## Out of scope

Alerts when a node dies (`node-down-alert`). Pinning actors to a node (`pinned-actors`). Node metrics in telemetry.
Changing the details of a running node: they are set at start.

## Order

After `conformance-fixtures`. Before `pinned-actors`.

## Impact

`crates/cicada` (`NodeConfig`, `NodeInfo`, both stores, an appended migration), the wire model in `api` and
`docs/openapi.json`, `cicada-client`, `cicada-app` (a setting, the node commands, the TUI, `docs/reference/cli.md`),
and a handoff to the docs session for `docs/operations.md` (Node table). Adding fields to `GET /nodes` changes a wire
format, so the design decides whether it needs an ADR. No guarantee changes.
