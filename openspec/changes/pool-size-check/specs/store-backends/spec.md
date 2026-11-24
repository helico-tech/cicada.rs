## ADDED Requirements

### Requirement: A node warns when its PostgreSQL pool is too small

A node that hosts at least one type SHALL, when it starts on a store whose pool is smaller than its concurrency plus
4, log one warning that names the pool size, the concurrency, the number it needs and the settings to change, and
then start as before. A store without a pool (SQLite, the in-memory store) SHALL never warn.

#### Scenario: A pool that is too small

- **WHEN** a node with concurrency 4 that hosts one type starts on a PostgreSQL store with a pool of 6
- **THEN** it logs one warning that names 6, 4 and 8 and the pool and concurrency settings, and it starts

#### Scenario: A pool that is large enough

- **WHEN** a node with concurrency 4 that hosts one type starts on a PostgreSQL store with a pool of 8
- **THEN** it logs no warning about the pool

#### Scenario: A node without types

- **WHEN** a node that hosts no types starts on a PostgreSQL store with a pool of 1
- **THEN** it logs no warning about the pool

#### Scenario: A store without a pool

- **WHEN** a node starts on a SQLite or in-memory store
- **THEN** it logs no warning about the pool
