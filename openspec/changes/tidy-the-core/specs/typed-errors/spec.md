# Spec Delta

## Purpose

A function of the core returns a named error and not a text. A caller can tell what stopped a node from starting,
and whether trying again can help, by the kind of the error and not by its text.

## ADDED Requirements

### Requirement: A function of the core returns a named error of its module

A public function of the core that returned a text, or a list of texts, as its error SHALL return a named error type
of its module: `RetryError`, `RetentionError`, `IdError`, `MessageError`, `ContractError`, `DefinitionError` or
`RequestError`. Each SHALL implement `std::error::Error`, hold its text and print the text that was returned before.
No public function of the core SHALL return a `String` as its error.

#### Scenario: A retry policy without attempts

- **WHEN** a retry policy is built with zero attempts
- **THEN** the error is a `RetryError` that prints "retry maxAttempts must be in 1..100, was 0"

#### Scenario: A blank message id

- **WHEN** a message id is built from a text of spaces only
- **THEN** the error is an `IdError` that prints "MessageId must not be blank"

#### Scenario: A retention of zero messages

- **WHEN** `Keep::last` is called with zero
- **THEN** the error is a `RetentionError` whose text names the count `0`

#### Scenario: A payload that does not decode

- **WHEN** a message of a known type is decoded from a payload with a field of the wrong kind
- **THEN** the error is a `MessageError` whose text names the message type

#### Scenario: A contract that is not an object

- **WHEN** a contract is read from the JSON value `5`
- **THEN** the error is a `ContractError` that prints "Not a contract: not an object"

#### Scenario: A malformed actor definition

- **WHEN** `contract_of` is asked for an actor whose ref and whose message family are both malformed
- **THEN** the error is a `DefinitionError` that holds two reasons and prints them joined with "; "

#### Scenario: An error read from JSON

- **WHEN** a retry policy with zero attempts is read from JSON
- **THEN** reading fails, and the message holds "retry maxAttempts must be in 1..100, was 0"

### Requirement: A start error holds a problem per cause

`StartError` SHALL hold one problem per cause. A contract conflict SHALL be a kind of its own that holds the type
name, the registered hash and the hash of this node. Every other problem SHALL hold its sentence. The kinds SHALL be
open to new ones. `StartError` SHALL be clonable and SHALL print the text it printed before.

#### Scenario: A contract conflict

- **WHEN** a node starts with a type whose contract differs from the registered one
- **THEN** the start fails with a problem of the kind contract conflict, which holds the type name and both hashes

#### Scenario: A store that fails while registering

- **WHEN** a node that hosts a type starts on a store that is closed
- **THEN** the start fails with one problem that holds its sentence, which names the actor and says that
  registering the contract failed

#### Scenario: Two problems at once

- **WHEN** a node starts with a setting that is out of range and a malformed actor definition
- **THEN** the start fails with two problems, each of which holds its sentence

#### Scenario: A node that is started twice

- **WHEN** `start` is called on a node that is already started
- **THEN** it fails with one problem that holds the sentence "Actor system is STARTED"

#### Scenario: The printed text of a conflict

- **WHEN** a start fails with a contract conflict and the error is printed
- **THEN** the line of that problem begins with the Rust type name of the actor and holds both hashes

### Requirement: A migration that violates a constraint is not reported as unavailable

When a statement of a migration violates a constraint of the database, the store SHALL fail the open with an
invalid-store error that names the schema version it could not reach and holds the database's message. A failure
that can pass, such as a lost connection, a locked database or a deadlock, SHALL stay "unavailable". Both SHALL hold
for SQLite and for PostgreSQL.

#### Scenario: Two keys that become one name

- **WHEN** a database of schema version 2 holds two actors whose keys become one name, and a store of this version
  opens it
- **THEN** the open fails with an invalid-store error that names schema version 3 and holds the database's message,
  not with "unavailable", and the database is still at schema version 2

#### Scenario: A database that is locked while it is migrated

- **WHEN** a SQLite database of an older schema version is locked by another writer, and a store of this version
  opens it
- **THEN** the open fails with "unavailable", and a later open, when the lock is gone, migrates the database
