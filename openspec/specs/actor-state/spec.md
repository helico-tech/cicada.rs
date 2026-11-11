# actor-state Specification

## Purpose
Every actor instance has a state value from its creation on. This spec says what a state may be, how an instance gets
one, and what becomes of a database that was written before states were required.

## Requirements

### Requirement: Every actor instance has a state

An actor instance SHALL have a state from its creation on. `GET /actors/{type}/{key}` SHALL always answer with a
`state`. Both stores SHALL refuse a row in `actors` without a state.

#### Scenario: An actor of a type that keeps no data

- **WHEN** an actor of a type whose state is `NoState` is created with `NoState {}` and a client gets it
- **THEN** the answer has `"state": {}`

#### Scenario: The store refuses a missing state

- **WHEN** a row without a state is inserted into `actors` with SQL, in SQLite and in PostgreSQL
- **THEN** the database refuses the insert

### Requirement: NoState is the state of a type that keeps no data

The library SHALL offer `NoState`, an empty struct, as the state of an actor type that keeps no data. It SHALL encode
to `{}`, and an object with a field SHALL NOT decode as `NoState`. There SHALL be no other kind of actor type:
`Stateless` and `create_stateless` do not exist.

#### Scenario: It encodes to an empty object

- **WHEN** `NoState {}` is encoded as a state
- **THEN** the result is `{}`

#### Scenario: A state with a field for a type that keeps no data

- **WHEN** a client sends `PUT /actors/maintenance.cleanup/_` with the body `{"state": {"oops": 1}}` to a node that
  hosts the type, whose state is `NoState`
- **THEN** the answer is `422 invalid-state` and no instance exists

### Requirement: A create names its state

Every way to create an instance SHALL take its initial state: a turn, `ActorSystem`, the client, HTTP and the command
line. `PUT /actors/{type}/{key}` SHALL answer `400` when its body is empty or has no `state`, and create nothing.
`cicada create actor` SHALL refuse to run without `--state`.

#### Scenario: A create over HTTP with a state

- **WHEN** a client sends `PUT /actors/maintenance.cleanup/_` with the body `{"state": {}}`
- **THEN** the answer is `201` and the instance has the state `{}`

#### Scenario: A create over HTTP without a body

- **WHEN** a client sends `PUT /actors/maintenance.cleanup/_` without a body
- **THEN** the answer is `400` and no instance exists

#### Scenario: A create over HTTP without the state field

- **WHEN** a client sends `PUT /actors/maintenance.cleanup/_` with the body `{}`
- **THEN** the answer is `400` and no instance exists

#### Scenario: The command line without a state

- **WHEN** `cicada create actor maintenance.cleanup/_` is run without `--state`
- **THEN** it exits with an error that names `--state`, and sends no request

#### Scenario: The command line with a state

- **WHEN** `cicada create actor maintenance.cleanup/_ --state '{}'` is run
- **THEN** the instance is created with the state `{}`

### Requirement: A state is never null

A state that is JSON `null` SHALL be refused. A create in a turn SHALL fail at the call with `TurnError::Invalid`. A
create from outside a turn SHALL be refused as an invalid state, over HTTP with `422 invalid-state`. A turn that ends
with a `null` state SHALL end its message as `failed`, without a retry, and the stored state SHALL stay as it was.

#### Scenario: A null state over HTTP

- **WHEN** a client sends `PUT /actors/counter/a` with the body `{"state": null}`
- **THEN** the answer is `422 invalid-state` and no instance exists

#### Scenario: A turn creates with a null state

- **WHEN** a handler calls `turn.create(&r, &())`
- **THEN** the call returns `TurnError::Invalid`, and the turn creates nothing unless the handler tries again

#### Scenario: A turn sets a null state

- **WHEN** a type has the state `Option<Draft>`, an instance holds a draft, and a handler sets the state to `None`
- **THEN** the message ends as `failed` without a retry, and the instance still holds the draft

### Requirement: A database of schema version 1 gets states

Opening a store on a database of schema version 1 SHALL bring it to version 2: every instance whose state is absent
or JSON `null` gets the state `{}`, every other state stays as it was, and from then on the database refuses a row
without a state. This SHALL hold for SQLite and for PostgreSQL.

#### Scenario: Instances with and without a state

- **WHEN** a database of schema version 1 holds one instance without a state, one with the state `null` and one with
  the state `{"n": 1}` at version 4, and a store of this version opens it
- **THEN** the states are `{}`, `{}` and `{"n": 1}`, the third instance is still at version 4, and the schema version
  is 2

#### Scenario: The indexes of SQLite survive

- **WHEN** a SQLite database of schema version 1 is opened by this version
- **THEN** the table `actors` has the indexes `actors_due` and `actors_running`, as a new database has
