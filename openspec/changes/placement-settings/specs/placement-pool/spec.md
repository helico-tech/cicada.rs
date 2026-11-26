## ADDED Requirements

### Requirement: An operator changes a pool's settings

A `Configure` message SHALL change any of a pool's silence, node limit, line limit and holder limit, within the bounds of
its create, and keep the others. A message with no setting or with a value out of bounds SHALL be refused for good and
change nothing. A limit lowered below what the pool has SHALL remove nothing and SHALL keep new nodes, holders or
waiting asks out until the count is below it. A new silence SHALL re-arm the silence check at the new earliest end.

#### Scenario: A lower holder limit

- **WHEN** a pool with three holders gets `maxHolders` 2 and then an ask that a node has room for
- **THEN** the three holders stay, the ask is queued, and it is granted only after two holders release

#### Scenario: A higher holder limit

- **WHEN** a pool at its holder limit with a waiting ask gets a higher `maxHolders`
- **THEN** the waiting ask is granted in that turn

#### Scenario: A shorter silence

- **WHEN** a live node last offered 50 seconds ago and the pool's silence changes from 60 to 30 seconds
- **THEN** the node is marked silent in the next turn and a `silent` alert goes out

#### Scenario: A message without a setting

- **WHEN** a `Configure` with no field set, or with `maxHolders` 0, reaches the pool
- **THEN** it is refused for good and the state is unchanged
