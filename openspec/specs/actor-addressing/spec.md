# actor-addressing Specification

## Purpose
TBD - created by archiving change actor-type-and-key. Update Purpose after archive.

## Requirements

### Requirement: Actors are addressed by type name and key

An actor address SHALL consist of a type name and a key, written `type/key`. A type name SHALL match
`[a-z][a-z0-9._-]*`. A key SHALL be one or more values joined with `:`, each percent-encoded, or `_` for a singleton.

#### Scenario: A ref with one field

- **WHEN** a ref of type `sync` has the field value `acme`
- **THEN** its address is `sync/acme`

#### Scenario: A ref with several fields

- **WHEN** a ref of type `batch-items` has the fields `batch` = `b1` and `index` = `7`, declared in that order
- **THEN** its address is `batch-items/b1:7`

#### Scenario: A ref without fields

- **WHEN** a ref of type `pricing` has no fields
- **THEN** its address is `pricing/_`

#### Scenario: A value holds a separator

- **WHEN** a field value is `a/b:c`
- **THEN** the key holds it percent-encoded as `a%2Fb%3Ac` and it decodes to the same value

#### Scenario: An invalid address

- **WHEN** an address has no key, has a key of `.` or `..`, or has a type name outside the allowed form
- **THEN** it is refused

### Requirement: An address determines its type

The type of an actor SHALL be the type name in its address. A registered type SHALL be found by that name. A second
registration of a name with a different contract SHALL conflict.

#### Scenario: Registering the same name with another contract

- **WHEN** a node registers type `sync` with a contract hash that differs from the registered one
- **THEN** the registration conflicts and the node does not start

#### Scenario: Changing a ref's fields

- **WHEN** the fields that form a type's key change
- **THEN** the contract hash changes

### Requirement: Actors are reached over HTTP by type and key

The HTTP API SHALL serve an actor at `/actors/{type}/{key}` and its mailbox at `/actors/{type}/{key}/messages`.

#### Scenario: An actor whose key is `messages`

- **WHEN** a client requests `/actors/sync/messages`
- **THEN** it addresses the actor `sync/messages`, not a mailbox

#### Scenario: A listing by prefix

- **WHEN** a client lists actors with `prefix=sync/`
- **THEN** only actors of type `sync` are returned
