# create-by-address

## ADDED Requirements

### Requirement: A turn can create an actor by its address

`Turn::create_raw` and `Turn::create_child_raw` SHALL buffer a create of the actor at an address with a state given
as JSON. They SHALL behave as `create` and `create_child` do: the create is idempotent, a state that is `null` is
refused at the call, and a child that exists under another parent rejects the commit. A call for an address whose
type is not registered SHALL fail with `TurnError::Invalid`, and one whose read of the registry fails with
`TurnError::Store`.

#### Scenario: A create by address

- **WHEN** a handler calls `create_raw` with the address `worker/w1` and the state `{"done": 0}`, and its turn commits
- **THEN** the actor `worker/w1` exists with that state and has no parent

#### Scenario: A child by address

- **WHEN** a handler calls `create_child_raw` with the address `worker/w1`, and the actor of the turn is later deleted
- **THEN** `worker/w1` is a child of that actor and is removed with it

#### Scenario: The actor exists

- **WHEN** a handler calls `create_raw` for an actor that exists
- **THEN** the turn commits and the actor keeps its state

#### Scenario: A type that is not registered

- **WHEN** a handler calls `create_raw` with an address whose type no node registered
- **THEN** the call returns `TurnError::Invalid`, and the handler can go on without it

#### Scenario: A null state

- **WHEN** a handler calls `create_raw` with the state `null`
- **THEN** the call returns `TurnError::Invalid`

### Requirement: A node that hosts the type checks the address and the state

On a node that hosts the type of the address, `create_raw` and `create_child_raw` SHALL check at the call that the
type can read the address and that the state decodes, and SHALL fail with `TurnError::Invalid` otherwise. A node
that does not host the type SHALL accept both as they are.

#### Scenario: A state that does not decode, on a hosting node

- **WHEN** a handler on a node that hosts `worker` calls `create_raw` with a state that has a field of the wrong kind
- **THEN** the call returns `TurnError::Invalid` and nothing is created

#### Scenario: An address the type cannot read

- **WHEN** a handler on a node that hosts the type calls `create_raw` with an address whose key the type's ref refuses
- **THEN** the call returns `TurnError::Invalid`

#### Scenario: The same state on a node that does not host the type

- **WHEN** a handler on a node that does not host `worker` calls `create_raw` with that state, and its turn commits
- **THEN** the actor exists with that state, and its first message ends `failed` with the cause `state`
