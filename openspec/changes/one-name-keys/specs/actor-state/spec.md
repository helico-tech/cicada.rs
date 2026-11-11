# Spec Delta

## MODIFIED Requirements

### Requirement: A database of schema version 1 gets states

Opening a store on a database of schema version 1 SHALL give every instance whose state is absent or JSON `null` the
state `{}`, leave every other state as it was, and from then on refuse a row without a state. The database SHALL end
at the schema version of the store that opened it. This SHALL hold for SQLite and for PostgreSQL.

#### Scenario: Instances with and without a state

- **WHEN** a database of schema version 1 holds one instance without a state, one with the state `null` and one with
  the state `{"n": 1}` at version 4, and a store of this version opens it
- **THEN** the states are `{}`, `{}` and `{"n": 1}`, the third instance is still at version 4, and the schema version
  is the one this store writes

#### Scenario: The indexes of SQLite survive

- **WHEN** a SQLite database of schema version 1 is opened by this version
- **THEN** the table `actors` has the indexes `actors_due` and `actors_running`, as a new database has
