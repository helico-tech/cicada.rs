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
