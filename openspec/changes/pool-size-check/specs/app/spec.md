## MODIFIED Requirements

### Requirement: The retention and the pool of the store can be set

`--retention` SHALL set how long the store keeps outcomes, and `--store-pool-size` the connections of the node on
PostgreSQL, each with a variable `CICADA_…` of the same name. Without the second, the pool SHALL be the larger of
the default of the store and the concurrency of the node plus 4. A given pool below the concurrency plus 4 SHALL be
used as given; the node's own check warns about it.

#### Scenario: The retention

- **WHEN** the binary runs `serve --retention 1h`, and then `serve --retention 14d`
- **THEN** the store is opened with a retention of one hour, and then of fourteen days

#### Scenario: The pool

- **WHEN** the binary runs `serve --store-pool-size 5` with a PostgreSQL store, and then without the flag at a
  concurrency of 40
- **THEN** the store is opened with 5 connections, and then with 44

#### Scenario: A given pool that is too small

- **WHEN** the binary runs `serve --store-pool-size 5 --concurrency 16` with a PostgreSQL store
- **THEN** the store is opened with 5 connections, not raised to 20

#### Scenario: The default pool is large enough

- **WHEN** the binary runs `serve` with a PostgreSQL store at any concurrency, without `--store-pool-size` or
  `CICADA_STORE_POOL_SIZE`
- **THEN** the pool is at least the concurrency plus 4
