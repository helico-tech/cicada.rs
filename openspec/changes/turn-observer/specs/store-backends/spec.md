# store-backends

## ADDED Requirements

### Requirement: A store that is open keeps working when a column is added

A store that is open and has run its statements SHALL keep working when a column with a default is added to a table
of its schema, on both SQL backends, so that a later migration that only adds such a column can be additive: nodes
of the release before keep running while it is applied.

#### Scenario: A column is added under an open store on PostgreSQL

- **WHEN** a store on PostgreSQL has created an actor, sent, claimed, committed and listed, a column with a default
  is then added to each of its tables on another connection, and the store does the same operations again
- **THEN** every operation succeeds as before

#### Scenario: A column is added under an open store on SQLite

- **WHEN** a store on SQLite has run the same operations, a column with a default is added to each of its tables, and
  it runs them again
- **THEN** every operation succeeds as before
