# node-view Specification

## Purpose
Every node records itself in a node table with a heartbeat, so an operator sees the nodes of a system
and whether each is alive, suspect or dead. The table is for display only: leases decide who runs what.

## Requirements

### Requirement: Every node records itself in a node table

Every started node, send-only nodes included, SHALL write a row in the node table with its id, hosted types and captured optional details,
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

#### Scenario: Captured details on a heartbeat

- **WHEN** a new-code node heartbeats
- **THEN** its hostname, core Cicada version and configured label replace those fields in its row, including None
- **AND** started_at keeps its existing meaning, and metadata does not affect turn claiming or node health

#### Scenario: Duplicate or mixed-version writer

- **WHEN** two nodes reuse one id
- **THEN** the existing shared-row behavior remains, including removal by either node's clean stop
- **AND** a new-code writer replaces details, while an old writer may leave another writer's metadata unchanged
- **AND** neither hostname nor label establishes unique or authenticated identity

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

### Requirement: Node details are captured without requiring hostname lookup to succeed

A started ActorSystem SHALL capture its OS hostname and core crate version once before spawning background tasks.
Failed, non-UTF-8, blank, NUL-containing or over-200-character hostnames SHALL be omitted with one debug diagnostic.
The runtime SHALL NOT truncate or substitute an environment hostname. A remote Worker SHALL add no node row.

#### Scenario: Lookup cannot provide a usable hostname

- **WHEN** lookup fails or returns unusable text
- **THEN** the node still starts with no hostname, the core version and its configured label
- **AND** the diagnostic does not contain the rejected value

#### Scenario: Details stay fixed for the node's life

- **WHEN** the operating-system hostname changes after start
- **THEN** subsequent heartbeats keep the captured hostname
- **AND** start schedules the initial heartbeat as before rather than promising its write completed before return

#### Scenario: Row-producing and row-free execution

- **WHEN** a send-only ActorSystem or direct-store App worker starts
- **THEN** it heartbeats with details
- **WHEN** a system only initializes or runs due turns, or a Worker runs over HTTP
- **THEN** this feature adds no node row or hostname lookup for it

### Requirement: A configured node label is bounded

NodeConfig SHALL default label to None. Existing configuration validation SHALL reject a present label that is blank
when trimmed, contains NUL or exceeds MAX_ID_LENGTH Unicode scalar values. Valid text SHALL be kept verbatim.

#### Scenario: Invalid labels

- **WHEN** a validated NodeConfig contains a blank, NUL-containing or 201-character label
- **THEN** configuration validation reports a StartProblem before background work starts

#### Scenario: Valid Unicode and absence

- **WHEN** label is None or nonblank NUL-free text of at most 200 Unicode scalar values
- **THEN** it passes label validation, including a 200-character Unicode value
- **AND** surrounding spaces in otherwise valid text are retained

### Requirement: Node metadata is optional in public views

Public NodeInfo and private NodeRecord SHALL carry optional hostname, cicada_version and label. NodeView SHALL expose hostname,
cicadaVersion and label, omit absent values and deserialize missing values as None. The flattened node detail SHALL
carry the same fields. Existing endpoints and host access control SHALL remain unchanged.

#### Scenario: New and old HTTP shapes

- **WHEN** GET /nodes or GET /nodes/{node} reads a row with details
- **THEN** the fields appear in its node view and the generated OpenAPI describes them as optional
- **WHEN** a row has no details or a new client decodes an old response
- **THEN** the fields are omitted or read as None respectively
- **AND** explicit null also reads as None, while old tolerant decoders ignore the added fields
