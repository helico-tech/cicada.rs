# store-counts Specification

## Purpose
An operator counts the actors and pending messages under an address prefix and sees the delivery
time of the oldest message that is due, so a backlog shows without a listing.

## Requirements

### Requirement: Counts under a prefix

`GET /stats?prefix=` SHALL answer `{actors, pending, oldestDue}` for the actors whose address starts with the prefix
(empty for all): the number of actors, the number of pending messages (including one being handled and delayed ones)
and `oldestDue`, the earliest `deliverAt` of a pending message that is due at store time, or null when none is due.

#### Scenario: An empty store

- **WHEN** nothing is stored
- **THEN** the answer is `{"actors": 0, "pending": 0, "oldestDue": null}`

#### Scenario: A prefix limits the count

- **WHEN** two actors of type `counter` and one of type `sync` exist, each with a pending message
- **THEN** `prefix=counter/` counts two actors and two messages, and `prefix=sync/` one and one

#### Scenario: Only delayed messages

- **WHEN** every pending message is delayed into the future
- **THEN** `pending` counts them and `oldestDue` is null

#### Scenario: The oldest due message

- **WHEN** two pending messages are due and a client asks for the stats
- **THEN** `oldestDue` is the earlier of their delivery times, and it moves to the later one once the earlier message is
  handled

### Requirement: The number of failed outcomes

`GET /stats?prefix=` SHALL also answer `failed`: the number of outcomes with status `failed` under the prefix that
`GET /outcomes` would list at that moment, by its visibility rule, read in the same read transaction as the other
numbers.

#### Scenario: Failed outcomes under a prefix

- **WHEN** two messages to `sync/` actors and one to a `counter/` actor failed, and one more `sync/` message is done
- **THEN** `prefix=sync/` answers `failed` 2, `prefix=counter/` answers 1, and no prefix answers 3

#### Scenario: Past retention

- **WHEN** a failed outcome is older than the retention and has no `visible_until` in the future
- **THEN** it is not listed by `GET /outcomes` and not counted in `failed`

#### Scenario: An older server

- **WHEN** the client reads `GET /stats` from a server that does not send `failed`
- **THEN** `StatsView::failed` is `None` and `cicada stats` shows `-`

#### Scenario: Hidden by its keep policy

- **WHEN** a message whose policy keeps no outcome (`Keep::Remove`) fails, within the store's retention
- **THEN** `GET /outcomes` does not list it and `failed` does not count it
