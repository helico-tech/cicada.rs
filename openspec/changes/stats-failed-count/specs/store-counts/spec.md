## ADDED Requirements

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
