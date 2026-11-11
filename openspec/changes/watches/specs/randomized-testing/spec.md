# Spec Delta

## ADDED Requirements

### Requirement: Notifications are written once under faults

The simulation of nodes under faults SHALL hold an actor that watches the others and is never removed, and SHALL
check that every `failed` outcome of a watched actor has exactly one `Failed` message for that watcher, pending or
finished, whatever commits were repeated or lost. Its check that every ended message was accepted SHALL count a
notification as accepted.

#### Scenario: A commit that is repeated

- **WHEN** the acknowledgement of a commit that ends a message as `failed` is lost, and the node commits again
- **THEN** the watcher has one `Failed` message for that message, not two

### Requirement: The comparison of backends covers watches

The comparison of the in-memory store with SQLite SHALL set and remove watches, in a commit and from outside, and
SHALL compare the stored watches and the notifications that each backend writes, with their ids and their order.

#### Scenario: A random list of operations with watches

- **WHEN** one random list of operations that sets watches, fails messages and removes actors runs against both
  backends
- **THEN** both hold the same watches, and the same notifications with the same ids in the same mailboxes
