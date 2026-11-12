# turn-management

## MODIFIED Requirements

### Requirement: An operator can cancel a running turn

`POST /actors/{type}/{key}/turn/cancel` SHALL record a request for the current turn, only while a valid lease is held,
and answer `202`, or `404 no-running-turn`. The owning node SHALL stop the handler at its next heartbeat and commit the
outcome `cancelled`, without effects and without retry, fenced by its epoch. An observer SHALL be told the end of the
turn with the outcome `cancelled`. A request SHALL never affect a later turn.

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
