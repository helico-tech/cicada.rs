# Spec Delta

## ADDED Requirements

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
