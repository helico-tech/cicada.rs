# Spec Delta

## ADDED Requirements

### Requirement: A contract always has a state schema

The contract of every type SHALL hold a JSON Schema of its state. In its JSON form and in the answers of `GET /types`
and `GET /types/{name}`, `state` SHALL be a schema, never `null` and never absent. The state schema SHALL count in the
contract hash, so a type that had no state in an earlier version has another hash in this one.

#### Scenario: A type that keeps no data

- **WHEN** a type whose state is `NoState` is registered and a client gets `/types/{name}`
- **THEN** `state` in the answer is the schema of an object without properties

### Requirement: A registration without a state schema is removed when the store is opened

Opening a store on a database of schema version 1 SHALL remove every registration whose contract has no state schema,
and SHALL keep every other registration and every actor. A node that hosts such a type SHALL start and register the
type again. An actor of a type that is not registered SHALL still be readable and deletable. This SHALL hold for
SQLite and for PostgreSQL.

#### Scenario: A stateless type is upgraded in place

- **WHEN** a database of schema version 1 holds the registration of the stateless type `maintenance.cleanup`, its
  actor `maintenance.cleanup/_` and a pending message for it, and a node of this version that hosts the type with the
  state `NoState` starts on it
- **THEN** the node starts, the type is registered with a state schema, the actor has the state `{}`, and the message
  is handled

#### Scenario: A type with state keeps its registration

- **WHEN** a database of schema version 1 holds the registration of a type that has a state schema, and a store of
  this version opens it
- **THEN** the registration is still there, with the same hash

#### Scenario: A type that no node hosts any more

- **WHEN** a database of schema version 1 holds the registration of a stateless type and one actor of it, and a node
  of this version that does not host the type opens it
- **THEN** the type is not registered, the actor can be read and has the state `{}`, and the actor can be deleted
