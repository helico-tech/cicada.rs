# sqlite-store Specification

## Purpose
How the SQLite store treats the database file it is given when it opens it: which files it sets up, which it migrates
and which it refuses.

## Requirements

### Requirement: The SQLite store refuses a database that is not its own

Opening a SQLite store SHALL fail with `StoreError::Invalid` when the database has no schema version and holds at least
one table that is not one of SQLite's own (`sqlite_` names). The message SHALL name the path and say that the database
holds other tables and is not a cicada store. The database SHALL be left exactly as it was.

#### Scenario: Another program's database

- **WHEN** a store is opened on a SQLite file that holds one table `foo` and no schema version
- **THEN** the open fails with `StoreError::Invalid` naming the path, and the file's bytes are unchanged

### Requirement: The SQLite store sets up a new or empty database

Opening a SQLite store on a path with no file, or on a database without tables and without a schema version, SHALL
create the store's tables, set the schema version and turn on WAL mode.

#### Scenario: A new path

- **WHEN** a store is opened on a path where no file exists
- **THEN** the file is created with the store's tables, the current schema version and WAL mode

#### Scenario: An empty file

- **WHEN** a store is opened on an existing file of zero bytes
- **THEN** the open succeeds and the file has the store's tables, the current schema version and WAL mode

#### Scenario: The store's own database

- **WHEN** a store is opened on a database the store set up before
- **THEN** the open succeeds and the data in it is kept
