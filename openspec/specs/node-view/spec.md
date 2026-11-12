# node-view Specification

## Purpose
Every node records itself in a node table with a heartbeat, so an operator sees the nodes of a system
and whether each is alive, suspect or dead. The table is for display only: leases decide who runs what.

## Requirements

### Requirement: Every node records itself in a node table

Every started node, send-only nodes included, SHALL write a row in the node table with its id and the types it hosts,
stamped with store time, at once and every 10 s. A clean stop SHALL remove the node's row. Any node's heartbeat SHALL
remove rows last seen more than a day ago. The table SHALL be for display only: no decision about who runs a turn may
use it.

#### Scenario: A send-only node has a row

- **WHEN** a node that hosts no types starts
- **THEN** `GET /nodes` lists it with empty `hosted`

#### Scenario: Stopping removes the row

- **WHEN** a started node stops
- **THEN** its row is gone from `GET /nodes`

#### Scenario: Old rows are removed

- **WHEN** a node's heartbeat runs and another row was last seen more than a day ago
- **THEN** that row is removed

### Requirement: Node status is derived from the age of the last heartbeat

`GET /nodes` SHALL list every node with `status` computed in the store's clock: `alive` while last seen under 30 s ago,
`suspect` under 2 minutes, otherwise `dead`, together with the number of its running turns. `GET /nodes/{node}` SHALL
answer one node with the list of its running turns, or `404` with problem type `no-such-node`.

#### Scenario: A node stops heartbeating

- **WHEN** a node last heartbeat 90 seconds ago
- **THEN** `GET /nodes` reports it `suspect`, and `dead` once 2 minutes have passed

#### Scenario: One node's turns

- **WHEN** a node runs a turn and a client gets `/nodes/{node}`
- **THEN** the answer lists that turn with its actor and message id

#### Scenario: An unknown node

- **WHEN** a client gets `/nodes/nope`
- **THEN** the answer is `404` with problem type `no-such-node`
