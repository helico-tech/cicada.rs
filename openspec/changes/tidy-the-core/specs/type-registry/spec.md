# Spec Delta

## ADDED Requirements

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
