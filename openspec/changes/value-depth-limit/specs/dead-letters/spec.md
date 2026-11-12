# dead-letters

## MODIFIED Requirements

### Requirement: Dead letters are told of failed messages by a watch

The dead-letters actor SHALL have the type name `cicada.dead-letters`. For every `Failed` it gets, it SHALL record a
letter with the id, the actor, the type, the payload, the correlation id, the sender, the error, the cause and the
attempts of the failed message, and the time it was recorded. It SHALL ignore a `Stopped`, a `Failed` about a
message of its own, and a `Failed` that the store did not mark as a notification. What it is told of is decided by the
watches that are set for it. A `Failed` whose payload is too deep to be kept in a letter (deeper than 61 levels)
SHALL fail the turn that would record it, with the cause `handler`, and no letter SHALL be recorded for it.

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

#### Scenario: The deepest payload a letter keeps

- **WHEN** a message with a payload 61 levels deep ends `failed`
- **THEN** the dead-letters actor holds a letter for it with that payload

#### Scenario: A payload too deep for a letter

- **WHEN** a message with a payload 62 levels deep ends `failed`
- **THEN** the turn of the dead-letters actor that gets its `Failed` ends `failed` with the cause `handler` and an
  error that names the depth, no letter is recorded, and the next failure is recorded as before

#### Scenario: A deep letter from before

- **WHEN** a dead-letters actor holds a letter, written before this release, whose payload is 70 levels deep, and a
  message ends `failed`
- **THEN** the turn of the dead-letters actor fails and records no letter; after an operator sends `Discard` for the
  deep letter, which ends `done`, the next failure is recorded
