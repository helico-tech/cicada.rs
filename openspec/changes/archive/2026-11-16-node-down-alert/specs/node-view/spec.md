## MODIFIED Requirements

### Requirement: Every node records itself in a node table

Every started node, send-only nodes included, SHALL write a row in the node table with its id, hosted types and captured optional details,
stamped with store time, at once and every 10 s. A clean stop SHALL remove the node's row. Any node's heartbeat SHALL
remove rows last seen more than a day ago. The table SHALL be for observability, including sampled alerts: no decision about who runs a turn may
use it. Node-down alerts SHALL NOT change row retention or establish a complete death history.

#### Scenario: A send-only node has a row

- **WHEN** a node that hosts no types starts
- **THEN** `GET /nodes` lists it with empty `hosted`

#### Scenario: Stopping removes the row

- **WHEN** a started node stops
- **THEN** its row is gone from `GET /nodes`

#### Scenario: Old rows are removed

- **WHEN** a node's heartbeat runs and another row was last seen more than a day ago
- **THEN** that row is removed

#### Scenario: Captured details on a heartbeat

- **WHEN** a new-code node heartbeats
- **THEN** its hostname, core Cicada version and configured label replace those fields in its row, including None
- **AND** started_at keeps its existing meaning, and metadata does not affect turn claiming or node health

#### Scenario: Duplicate or mixed-version writer

- **WHEN** two nodes reuse one id
- **THEN** the existing shared-row behavior remains, including removal by either node's clean stop
- **AND** a new-code writer replaces details, while an old writer may leave another writer's metadata unchanged
- **AND** neither hostname nor label establishes unique or authenticated identity
