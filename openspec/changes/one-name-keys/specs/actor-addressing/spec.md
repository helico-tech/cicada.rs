# Spec Delta

## MODIFIED Requirements

### Requirement: Actors are addressed by type name and key

An actor address SHALL consist of a type name and a key, written `type/key`. A type name SHALL match
`[a-z][a-z0-9._-]*`. A key SHALL be one name, percent-encoded, or `_` for a singleton. A `:` in a name SHALL be
written as it is: it does not separate parts.

#### Scenario: A ref with one field

- **WHEN** a ref of type `sync` has the field value `acme`
- **THEN** its address is `sync/acme`

#### Scenario: A ref with several fields

- **WHEN** a node hosts a type whose ref has the fields `batch` and `index`
- **THEN** the node does not start, and the error names the ref

#### Scenario: A ref that joins two parts

- **WHEN** a ref of type `batch-items` builds its name from the batch `b1` and the index `7` as `b1.7`
- **THEN** its address is `batch-items/b1.7`

#### Scenario: A ref without fields

- **WHEN** a ref of type `pricing` has no fields
- **THEN** its address is `pricing/_`

#### Scenario: A value holds a separator

- **WHEN** a name is `a/b:c`
- **THEN** the key holds it as `a%2Fb:c`, with the slash encoded and the colon as it is, and it decodes to the same
  name

#### Scenario: An address of an old key with two parts

- **WHEN** the address `account/acme:7` was written for a ref with the fields `acme` and `7`, and the ref now has
  the one name `acme:7`
- **THEN** the address of the ref is `account/acme:7`, the same actor

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

- **WHEN** the one field of a type's ref gets another name
- **THEN** the contract hash stays the same

## ADDED Requirements

### Requirement: A ref is written as one string or as a unit

A ref SHALL be a type that serde writes as one string or as a unit: a struct of which serde writes one field, whose
value is a string; a type that serde writes as a string itself; or a unit struct. A field that serde skips SHALL NOT
count.

#### Scenario: A field that serde skips

- **WHEN** a ref has a `name` that is a `String` and a second field that serde skips, and the name is `nightly`, for
  the type `cron`
- **THEN** its address is `cron/nightly`, and a node that hosts the type starts

#### Scenario: A field that is a newtype of a string

- **WHEN** the one field of a ref is a newtype around a `String` that holds `acme`, for the type `sync`
- **THEN** its address is `sync/acme`

#### Scenario: A newtype ref

- **WHEN** a ref is a newtype around the `String` `acme`, for the type `sync`
- **THEN** its address is `sync/acme`

#### Scenario: A ref with typed parts that writes itself as a string

- **WHEN** a ref has the fields `batch` and `index` and names `String` as its serde form, writing `b1` and `7` as
  `b1.7`, for the type `batch-items`
- **THEN** its address is `batch-items/b1.7`, the address `batch-items/b1.7` reads back as the same ref, and a node
  that hosts the type starts

### Requirement: A ref that is not one string is refused

A number, a boolean, a single character and an enum variant SHALL NOT count as a string. A node that hosts a type
whose ref is not read as one string or a unit SHALL refuse to start and name the ref. A ref that is not written as
one string or a unit SHALL have no address, so that a send to it fails at the call.

#### Scenario: A ref with a number

- **WHEN** a node hosts a type whose ref has the one field `id` of a number type
- **THEN** the node does not start, and the error names the ref

#### Scenario: A ref with an enum variant

- **WHEN** a node hosts a type whose ref has the one field `region` of an enum without data
- **THEN** the node does not start, and the error names the ref

#### Scenario: A ref of a type the node does not host

- **WHEN** a turn sends to a ref with two fields whose type the node does not host
- **THEN** the send fails at the call, and the turn can go on

### Requirement: The views of an actor and of a type hold no parts of a key

The answer of `GET /actors/{type}/{key}` SHALL NOT hold `params`, and the answers of `GET /types` and
`GET /types/{name}` SHALL NOT hold `key`.

#### Scenario: The view of an actor

- **WHEN** a client gets `/actors/sync/acme`
- **THEN** the answer holds `address` and `type`, and no `params`

#### Scenario: The view of a type

- **WHEN** a client gets `/types/sync`
- **THEN** the answer holds `name`, `hash`, `state` and `messages`, and no `key`

### Requirement: Stored addresses get the one spelling when a database of schema version 1 or 2 is opened

Opening a store on a database of schema version 1 or 2 SHALL rewrite `%3A` to `:` in the address of every actor, and
in the address by which every pending message and every stored outcome is found. An address that is read
from the store SHALL have the one spelling, wherever it was stored. This SHALL hold for SQLite and for PostgreSQL.

#### Scenario: An old literal colon

- **WHEN** a database of schema version 2 holds the actor `notes/a%3Ab`, a pending message for it and an outcome of
  an earlier message, and a store of this version opens it
- **THEN** the actor is `notes/a:b`, its mailbox holds the pending message, its outcome is found under `notes/a:b`,
  and a node that hosts the type handles the message

#### Scenario: A sender with a literal colon

- **WHEN** a database of schema version 2 holds a pending message whose sender is `notes/a%3Ab`
- **THEN** the message that a store of this version reads has the sender `notes/a:b`

#### Scenario: Two keys that become one name

- **WHEN** a database of schema version 2 holds the actors `pairs/a%3Ab:c` and `pairs/a:b%3Ac`, and a store of this
  version opens it
- **THEN** the open fails, and the database is still at schema version 2 with both actors
