# Spec Delta

## Purpose

What happens to a message that an actor cannot decode: its type name is not one of the actor's, or its payload does
not fit. There is one rule, and no policy to choose.

## ADDED Requirements

### Requirement: A message that cannot be decoded ends as failed

A message whose type name is not one of the actor's message types, or whose payload does not decode, SHALL end as
`failed` with the reason, without a retry. No handler SHALL run for it, and the actor's state SHALL stay as it was.
An actor definition SHALL NOT state a policy for it, and a message family SHALL have no way to take a message with
a type name it does not declare.

#### Scenario: An unknown type name

- **WHEN** a message with the type name `Nope` is in the mailbox of an actor that has no such message type
- **THEN** the message ends as `failed`, the reason names `Nope`, and the actor's state is unchanged

#### Scenario: A payload that does not fit

- **WHEN** a message of a known type is in the mailbox with a payload that does not decode
- **THEN** the message ends as `failed` with the reason, after one attempt, although its sender set a retry policy

#### Scenario: The next message

- **WHEN** a message that cannot be decoded is followed by one that can
- **THEN** the second message is handled

### Requirement: A payload that is refused at the door says why once

A node that hosts the type of the target SHALL refuse a send over HTTP whose payload does not decode with `422`,
and the detail SHALL hold the reason once.

#### Scenario: A send with a payload that does not fit

- **WHEN** a client sends a message of the type `Add` whose payload is an object with a field of the wrong kind, to
  a node that hosts the target's type
- **THEN** the answer is `422`, and its detail holds the words "cannot be decoded" exactly once
