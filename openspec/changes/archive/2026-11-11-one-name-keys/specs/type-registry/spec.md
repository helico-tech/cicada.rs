# Spec Delta

## REMOVED Requirements

### Requirement: A registration without a state schema is removed when the store is opened

**Reason**: Every registration is now removed when a database that an earlier version wrote is opened, not only the
ones without a state schema. Its scenario "A type with state keeps its registration" is no longer true.

**Migration**: The requirement "Every registration is removed when a database of schema version 1 or 2 is opened"
replaces it and keeps its other two scenarios.

## ADDED Requirements

### Requirement: A contract holds no key fields

The contract of a type SHALL NOT hold the fields of its key: not in its JSON form and not in its hash. A contract
that an earlier version wrote, with a `key` entry, SHALL still be read, as the same contract without it.

#### Scenario: The JSON form

- **WHEN** a type is registered and its contract is read as JSON
- **THEN** the JSON holds `type`, `state` and `messages`, and no `key`

#### Scenario: A contract snapshot of an earlier version

- **WHEN** a contract snapshot file holds `"key": ["tenant"]` and nothing else of the type changed, and the snapshot
  check runs without the update flag
- **THEN** the check passes

### Requirement: Every registration is removed when a database of schema version 1 or 2 is opened

Opening a store on a database of schema version 1 or 2 SHALL remove every registration, and SHALL keep every actor
and every pending message. A node that hosts a type SHALL start on it and register the type again. An actor of
a type that is not registered SHALL still be readable and deletable. This SHALL hold for SQLite and for PostgreSQL.

#### Scenario: A stateless type is upgraded in place

- **WHEN** a database of schema version 1 holds the registration of the stateless type `maintenance.cleanup`, its
  actor `maintenance.cleanup/_` and a pending message for it, and a node of this version that hosts the type with the
  state `NoState` starts on it
- **THEN** the node starts, the type is registered with a state schema, the actor has the state `{}`, and the message
  is handled

#### Scenario: A type with state is upgraded in place

- **WHEN** a database of schema version 2 holds the registration of the type `counter`, an actor of it with a state
  and a pending message for that actor, and a node of this version that hosts the type starts on it
- **THEN** the node starts, the type is registered with a contract without key fields, the actor has its state, and
  the message is handled

#### Scenario: A type that no node hosts any more

- **WHEN** a database of schema version 2 holds the registration of a type and one actor of it, and a node of this
  version that does not host the type opens it
- **THEN** the type is not registered, the actor can be read, and the actor can be deleted

#### Scenario: A turn creates an actor of a type that is not registered yet

- **WHEN** a type is not registered because no node that hosts it has started on the upgraded database, and a turn
  on another node creates an actor of it
- **THEN** that turn's message ends as `failed`, without a retry
