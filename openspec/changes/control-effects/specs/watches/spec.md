# Spec Delta

## MODIFIED Requirements

### Requirement: A watcher is told when a watched actor goes away

When an actor that is watched by its address is stopped by its own turn, deleted from outside a turn or by the turn
of another actor, or removed with its parent, the same transaction SHALL put one `Stopped` message into the mailbox
of each watcher. It SHALL hold the actor, the cause (`stopped`, `deleted` or `parent`) and the actor whose turn
removed it, which is absent for a removal from outside a turn.

#### Scenario: An actor stops itself

- **WHEN** a turn of a watched actor stops it
- **THEN** each watcher has one `Stopped` message with the cause `stopped`, `by` is the actor itself, and its
  `causedBy` is the message whose turn stopped it

#### Scenario: An actor is deleted

- **WHEN** a watched actor is deleted over HTTP
- **THEN** each watcher has one `Stopped` message with the cause `deleted`, and `by` has the value null

#### Scenario: An actor is deleted by a turn

- **WHEN** a turn of another actor deletes a watched actor
- **THEN** each watcher has one `Stopped` message with the cause `deleted`, `by` is the actor whose turn deleted it,
  and its `causedBy` is the message of that turn

#### Scenario: A delete wakes the watcher

- **WHEN** a watched actor is deleted on a node that hosts its watcher, on virtual time
- **THEN** the watcher handles the `Stopped` at once, not after the next poll

#### Scenario: A child is removed with its parent

- **WHEN** a parent is deleted and its child is watched by a third actor
- **THEN** that actor has one `Stopped` message for the child, with the cause `parent`

#### Scenario: A child is removed with a parent that stops

- **WHEN** a turn of a parent stops it, and its child is watched by a third actor
- **THEN** that actor has one `Stopped` message for the child, with the cause `parent`, and `by` is the parent

#### Scenario: A watcher that is removed with its target

- **WHEN** a parent watches its child and the parent is deleted
- **THEN** no notification is written for the parent, and its watch is gone
