## ADDED Requirements

### Requirement: A turn can watch an actor that must exist

`Turn::watch_existing(target)` SHALL take one actor and, at the commit, set a watch of it when it exists, and
otherwise put one `Stopped` (cause `deleted`, `by` null, the turn's message as `causedBy`) into the watcher's mailbox
in the same transaction and leave the pair without a watch. A type prefix or `*` SHALL fail with `TurnError::Invalid`.

#### Scenario: The target is there

- **WHEN** a turn calls `watch_existing` for an actor that exists, and that actor is deleted later
- **THEN** the watch is set, and the watcher gets one `Stopped` from the delete

#### Scenario: The target is already gone

- **WHEN** a turn calls `watch_existing` for an actor that was deleted before the commit
- **THEN** the watcher gets one `Stopped` with the cause `deleted` and `by` null, and no watch is stored

#### Scenario: A pool place is freed

- **WHEN** a pool gives a place to a run and sends it a message, the run is deleted before the pool's next turn, and
  that turn calls `watch_existing` for the run
- **THEN** the pool handles a `Stopped` for the run and frees the place

#### Scenario: A type is refused

- **WHEN** a turn calls `watch_existing` with the target `billing`
- **THEN** the call fails with `TurnError::Invalid` and the turn can go on

### Requirement: The check of an actor that must exist misses no delete

The commit SHALL check a `watch_existing` target where it applies watches, in the order of the calls and before the
turn's deletes and creates, so that a delete of the target by another transaction gives exactly one `Stopped`.

#### Scenario: A delete races the commit

- **WHEN** on PostgreSQL another node deletes the target while a turn that calls `watch_existing` for it commits
- **THEN** the watcher gets exactly one `Stopped`

#### Scenario: Created in the same turn

- **WHEN** a turn creates an actor and calls `watch_existing` for it
- **THEN** the watcher gets a `Stopped`, as the check comes before the turn's creates

#### Scenario: Calls in one turn

- **WHEN** a turn calls `watch_existing` twice for an absent actor, or `watch` and then `watch_existing` for it
- **THEN** the watcher gets one `Stopped`, and no watch of that pair is stored
