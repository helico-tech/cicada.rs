# Spec Delta

## ADDED Requirements

### Requirement: A store on PostgreSQL can be opened over TLS

With the feature `postgres`, the core SHALL be built with TLS for PostgreSQL, with nothing else to switch on. A URL
that asks for TLS SHALL NOT fail because the build lacks TLS. The feature `sqlite` alone SHALL NOT build a TLS
library for the store.

#### Scenario: TLS is asked from a server that offers none

- **WHEN** a store is opened with `sslmode=require` on a PostgreSQL server that offers no TLS
- **THEN** the open fails, and the error says that the server does not support TLS, not that the build lacks it

#### Scenario: A URL without a mode

- **WHEN** a store is opened on the same server with a URL that names no `sslmode`
- **THEN** the open succeeds, as before

#### Scenario: A SQLite host

- **WHEN** the core is built with the feature `sqlite` alone
- **THEN** `cargo tree` for that build lists no TLS library
