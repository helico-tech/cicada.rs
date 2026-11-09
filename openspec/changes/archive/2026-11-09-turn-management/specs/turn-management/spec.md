## ADDED Requirements

### Requirement: Running turns can be listed

`GET /turns?node=&prefix=&cursor=&limit=` SHALL list the instances with a lease valid at store now, in address order:
the actor, the message being handled, the node holding the lease, its expiry and whether a cancel was requested. `GET
/actors/{type}/{key}/turn` SHALL answer that one turn, or `404` with problem type `no-running-turn`. Both are read routes.

#### Scenario: Filtered by node

- **WHEN** two nodes each run a turn and a client gets `/turns?node=n1`
- **THEN** only the turn of `n1` is listed

#### Scenario: No running turn

- **WHEN** no valid lease is held on an actor
- **THEN** `GET /actors/{type}/{key}/turn` answers `404` with problem type `no-running-turn`

### Requirement: An operator can cancel a running turn

`POST /actors/{type}/{key}/turn/cancel` SHALL record a request for the current turn, only while a valid lease is held,
and answer `202`, or `404 no-running-turn`. The owning node SHALL stop the handler at its next heartbeat and commit the
outcome `cancelled`, without effects and without retry, fenced by its epoch. `on_outcome` SHALL see the outcome and
`on_handler_error` SHALL not be called. A request SHALL never affect a later turn.

#### Scenario: A handler is cancelled

- **WHEN** a turn is running and a client posts `/turn/cancel`
- **THEN** within one heartbeat the handler stops and the message ends `cancelled` with no effects, and the mailbox moves
  on to the next message

#### Scenario: The handler already ended

- **WHEN** the handler has already returned and the node commits while a cancel request arrives
- **THEN** the request is ignored and the turn commits as usual (G6)

#### Scenario: Blocking code

- **WHEN** a cancel request arrives while a handler blocks its thread
- **THEN** the handler runs until it returns, and the commit is `cancelled` rather than done

#### Scenario: A later turn

- **WHEN** a request was made and that turn ended, and a later turn of the same actor runs
- **THEN** the later turn is not affected

### Requirement: There is no forced release of a lease

The API SHALL NOT offer any way to take a lease away from a node: on a live node it would break G1 and G6, and a dead
node's lease expires by itself within the lease TTL.

#### Scenario: No force option

- **WHEN** a client looks through the HTTP API for a way to force a lease away
- **THEN** there is none; `turn/cancel` is the only way to stop a running turn
