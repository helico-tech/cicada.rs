# dead-letters Specification

## Purpose
The dead-letters actor collects failed messages through a watch, with no polling. An operator sends a letter
again once, or forgets it.

## Requirements

### Requirement: Dead letters are told of failed messages by a watch

The dead-letters actor SHALL have the type name `cicada.dead-letters`. For every `Failed` it gets, it SHALL record a
letter with the id, the actor, the type, the payload, the correlation id, the sender, the error, the cause and the
attempts of the failed message, and the time it was recorded. It SHALL ignore a `Stopped`, a `Failed` about a
message of its own, and a `Failed` that the store did not mark as a notification. What it is told of is decided by the
watches that are set for it.

#### Scenario: Every failure

- **WHEN** a watch on `*` is set for a dead-letters actor, and a message to any actor ends `failed`
- **THEN** its state holds a letter for that message, with its payload, its error and its cause

#### Scenario: A `Failed` sent by a client

- **WHEN** a client sends a dead-letters actor a `Failed`
- **THEN** the message ends `done`, and nothing changes: no letter is recorded

#### Scenario: One type

- **WHEN** the watch is on the target `billing`, and messages to `billing.invoices/a` and to `shipping/b` end `failed`
- **THEN** it holds a letter for the first and none for the second

#### Scenario: A failure of its own

- **WHEN** a dead-letters actor with a watch on `*` fails a message of its own
- **THEN** it records no letter for it

#### Scenario: No delay

- **WHEN** a message ends `failed`
- **THEN** the letter is recorded by the next turn of the dead-letters actor, with no timer in between

### Requirement: Dead letters keep at most their capacity

The dead-letters actor SHALL keep at most its capacity of letters, 1,000 unless the type is built with another, and
SHALL drop the letters that were recorded first.

#### Scenario: More letters than the capacity

- **WHEN** a dead-letters actor with a capacity of 2 records a third letter
- **THEN** it holds the second and the third

### Requirement: A letter can be sent again once, or forgotten

`Redrive` SHALL send the failed message of a letter again, to the same actor, with the same type, payload and
correlation id, and SHALL record the id of the new message in the letter. A letter that was sent again SHALL NOT be
sent a second time. When the type of the actor no longer takes the message, nothing SHALL be sent and the letter
SHALL record why. `Discard` SHALL forget a letter.

#### Scenario: A redrive

- **WHEN** an operator sends `Redrive` for a letter whose actor exists
- **THEN** the actor gets a new message with the payload of the failed one, and the letter holds its id

#### Scenario: A second redrive

- **WHEN** an operator sends `Redrive` again for that letter
- **THEN** nothing is sent

#### Scenario: A redrive that fails again

- **WHEN** the message of a redrive ends `failed` too
- **THEN** there is a second letter, for the new message

#### Scenario: A type that no longer takes the message

- **WHEN** an operator sends `Redrive` for a letter whose message type the actor's type no longer has
- **THEN** nothing is sent, and the letter records why

#### Scenario: The actor is gone

- **WHEN** an operator sends `Redrive` for a letter whose actor does not exist
- **THEN** the `Redrive` ends `failed` with the cause `commit`, and the letter is as it was

#### Scenario: A discard

- **WHEN** an operator sends `Discard` for a letter
- **THEN** the state no longer holds it

#### Scenario: A letter that is not there

- **WHEN** an operator sends `Redrive` or `Discard` for an id that is no letter
- **THEN** nothing changes and the message ends `done`
