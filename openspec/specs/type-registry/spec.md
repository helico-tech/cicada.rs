# type-registry Specification

## Purpose
TBD - created by archiving change remove-registered-type. Update Purpose after archive.

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
