# type-registry Specification

## Purpose
The store keeps the contract of each registered type: its schemas, compared by a hash that leaves
out the descriptions. An operator removes the registration of a type that has no actors, and a send to an
actor of a type that is not registered answers that there is no such type.

## Requirements

### Requirement: A type's registration can be removed

`DELETE /types/{name}` SHALL remove the registration of the type and answer `204`. It SHALL answer `404` with the problem
type `no-such-type` when no type has that name, and `409` with the problem type `type-in-use` while actors of the type
exist, leaving the registration as it was. The check and the removal SHALL be one atomic step. It SHALL be a write route:
the read-only router does not serve it. The type SHALL NOT be served after its removal, by lookups or by creates.

#### Scenario: An unused type

- **WHEN** no actor of the type `counter` exists and a client deletes `/types/counter`
- **THEN** the answer is `204`, `GET /types/counter` answers `404`, and a create of a `counter` actor answers `404`

#### Scenario: A type in use

- **WHEN** an actor `counter/a` exists and a client deletes `/types/counter`
- **THEN** the answer is `409 type-in-use` and the type stays registered

#### Scenario: An unknown type

- **WHEN** a client deletes `/types/nope`
- **THEN** the answer is `404 no-such-type`

#### Scenario: A create races the removal

- **WHEN** an actor is created while its type is removed
- **THEN** either the create finishes first and the removal is refused with `409`, or the create finds the type gone and
  answers `404`; no actor stays behind without its type

#### Scenario: Registering again

- **WHEN** a type was removed and a node that hosts it starts
- **THEN** the node registers its contract again

### Requirement: A contract hash ignores schema descriptions

A contract's hash SHALL be computed from its JSON form with every `description` keyword removed from the state schema
and the message schemas, at any depth where a schema can sit. A property, definition or message type that is named
`description` SHALL still count. Two contracts that differ only in descriptions SHALL have the same hash.

#### Scenario: A doc comment changes

- **WHEN** only the doc comment of a field or a type of a contract changes
- **THEN** the contract hash stays the same, and a node with the new comment starts against a store where the old one
  is registered

#### Scenario: A nested description

- **WHEN** only a `description` inside a nested schema changes (a property of a property, an item of a list, a branch of
  `anyOf`, or an entry of `$defs`)
- **THEN** the contract hash stays the same

#### Scenario: A property named description

- **WHEN** a schema gains, loses or changes a property named `description`
- **THEN** the contract hash changes

### Requirement: The published contract keeps its descriptions

The JSON form of a contract, as stored by the registry and served by `GET /types` and `GET /types/{name}`, SHALL keep
every `description` of its schemas. The registry SHALL keep the JSON form of the first registration when a node
registers a contract with the same hash.

#### Scenario: Served with descriptions

- **WHEN** a type whose fields have doc comments is registered and a client gets `/types/{name}`
- **THEN** the schemas in the answer carry those doc comments as `description`

#### Scenario: A comment changes after registration

- **WHEN** a node with a changed doc comment registers a type that is already registered
- **THEN** the registration succeeds and `GET /types/{name}` still serves the descriptions of the first registration

### Requirement: A contract always has a state schema

The contract of every type SHALL hold a JSON Schema of its state. In its JSON form and in the answers of `GET /types`
and `GET /types/{name}`, `state` SHALL be a schema, never `null` and never absent. The state schema SHALL count in the
contract hash, so a type that had no state in an earlier version has another hash in this one.

#### Scenario: A type that keeps no data

- **WHEN** a type whose state is `NoState` is registered and a client gets `/types/{name}`
- **THEN** `state` in the answer is the schema of an object without properties

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

### Requirement: A send to an actor of a type that is not registered answers no such type

A send from outside a turn to an actor whose type is not registered SHALL be refused as "no such type", whether or
not a row of that actor exists. Over HTTP that SHALL be `404 no-such-type`. A send to an actor that does not exist,
of a type that is registered, SHALL stay `404 no-such-instance`. A send with an id whose message is pending or whose
outcome is still shown SHALL be answered as a duplicate, whatever became of the type.

#### Scenario: The type is not registered

- **WHEN** an actor exists whose type is not registered, and a client sends it a message over HTTP
- **THEN** the answer is `404 no-such-type`

#### Scenario: The type is registered and the actor is missing

- **WHEN** a client sends a message to `counter/nope`, and the type `counter` is registered and has no such actor
- **THEN** the answer is `404 no-such-instance`

#### Scenario: A repeated send after the type was removed

- **WHEN** a message was handled and its outcome is still shown, its actor was deleted and its type removed, and the
  send is repeated with the same id
- **THEN** the answer is a duplicate, not "no such type"

### Requirement: A retry of a failed message does not need the type to be registered

A retry of a failed message SHALL NOT count as a send from outside a turn: it SHALL be accepted whether or not the
type of its actor is registered, and the message SHALL wait in the mailbox until a node that hosts the type starts.

#### Scenario: A retry while the type is not registered

- **WHEN** an actor exists whose type is not registered, and a client retries a failed message of that actor
- **THEN** the retry is accepted, and the message waits in the mailbox

### Requirement: A message schema holds every definition it refers to

The schema of each message type of a contract SHALL hold every definition that it refers to, also one whose name is
escaped in a reference: a name with `~`, `/`, a space, `<`, `>`, another character outside the safe set of a
fragment, or a letter that is not ASCII, whether the name comes from `#[schemars(rename)]`, from a container
`#[serde(rename)]` or from the Rust type. A reference of several segments SHALL need the definition of its first
segment.

#### Scenario: A renamed type

- **WHEN** a message holds a field of a type that is renamed to `Line <v2>` with `#[schemars(rename)]`
- **THEN** the schema of that message type holds the definition `Line <v2>`, and the door checks a send of it

#### Scenario: A type renamed for serde

- **WHEN** a message holds a field of a type with the container attribute `#[serde(rename = "line/v2")]`
- **THEN** the schema of that message type holds the definition `line/v2`

#### Scenario: A type name with a letter that is not ASCII

- **WHEN** a message holds a field of the type `Größe`
- **THEN** the schema of that message type holds the definition `Größe`

#### Scenario: A newtype variant of a renamed type

- **WHEN** a variant of a message family is a newtype of a renamed type with a space in its name
- **THEN** the node starts, and the schema of that message type is the schema of that type, with the definitions it
  refers to

#### Scenario: A newtype variant whose definition is missing

- **WHEN** a hand-written family schema has a newtype variant that refers to a definition the schema does not hold
- **THEN** the error names the variant and the missing definition

#### Scenario: A reference of several segments

- **WHEN** a hand-written message schema refers to `#/$defs/Line/properties/sku`
- **THEN** the schema of that message type holds the definition `Line`

#### Scenario: A type that is not hit

- **WHEN** the contract of a type whose definition names need no escaping, whose family does not hold itself, and
  whose references have one segment, is made
- **THEN** it is the same as before, byte for byte

### Requirement: A newtype variant refers to a whole definition

A newtype variant of a message family SHALL refer to a whole definition, so that the door is never stricter than the
decoder. A family with a newtype variant whose reference points inside a definition SHALL be refused, with an error
that names the variant and the reference. A reference of one segment SHALL NOT be affected.

#### Scenario: A newtype variant that points inside a definition

- **WHEN** a hand-written family schema has a newtype variant that refers to `#/$defs/Line/properties/sku`
- **THEN** the error names the variant and the reference, and no schema of the family is made

### Requirement: A family that holds itself is never refused for its nested messages

When a message family holds itself, a reference to the whole family in the schema of one message type SHALL accept
any value, so that the door never refuses a valid nested message; the decoder checks it. The schema of a message
type that holds no such reference SHALL NOT change.

#### Scenario: A batch of messages

- **WHEN** a family has a message `Batch { items: Vec<Msg> }` and a client sends a `Batch` whose items are an `Add`
  and a `Batch`
- **THEN** the door accepts it on every node, as the type decodes it

#### Scenario: A nested message that is wrong, on a node that hosts the type

- **WHEN** an item of such a `Batch` has a field of the wrong kind, and the send goes to a node that hosts the type
- **THEN** the answer is `422`, from the decoder

#### Scenario: A nested message that is wrong, on a node that does not host the type

- **WHEN** the same send goes to a node that does not host the type
- **THEN** it is accepted, and the message ends `failed` with the cause `message` in its turn

#### Scenario: The other messages of the family

- **WHEN** a family has `Batch` and `Add`, and only `Batch` refers to the family
- **THEN** the schema of `Add` is the same as before, byte for byte

#### Scenario: A `#` that is data

- **WHEN** a hand-written schema of a message holds `"#"` as a `const` value
- **THEN** that value stays as it is
