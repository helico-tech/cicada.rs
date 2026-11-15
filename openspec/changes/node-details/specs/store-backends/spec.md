## ADDED Requirements

### Requirement: Node metadata uses an additive migration

Both SQL stores SHALL append migration 7 adding nullable text hostname, cicada_version and label columns to nodes.
Existing rows SHALL retain their previous data with absent metadata. Already-open schema-6 writers SHALL keep
working with their explicit column lists; opening a newer schema with an old binary SHALL remain refused.

#### Scenario: Upgrade an existing node table

- **WHEN** a schema-6 database with node rows is opened by the new implementation
- **THEN** migration 7 preserves those rows and reads all three new fields as None
- **AND** a new heartbeat replaces all details, including clearing a previously present label
- **AND** the same details and overwrite-to-None semantics pass conformance on memory, SQLite and PostgreSQL

#### Scenario: A running old connection survives the additive migration

- **GIVEN** an open connection with prepared schema-6 node read and heartbeat statements
- **WHEN** another connection applies migration 7
- **THEN** the old prepared statements still read, insert and update using the original column lists
- **AND** old inserts leave metadata NULL, while old updates do not change existing metadata

#### Scenario: Old binaries cannot reopen or roll back

- **WHEN** a schema-6 implementation tries to open the migrated database
- **THEN** the existing newer-schema guard refuses it
- **AND** rollout instructions state that already-running old nodes may continue but cannot restart after migration
