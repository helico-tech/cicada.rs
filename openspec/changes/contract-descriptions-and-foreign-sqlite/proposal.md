# Proposal

## Why

Chorus, a user of this library, found two problems (its change `cicada-requests`, points 1, 7 and 10; its dogfood
issues 0003 and 0014):

1. **A doc comment changes a contract.** A contract's hash is the SHA-256 of the canonical JSON of the type's schemas.
   schemars writes doc comments into those schemas as `description`. So fixing a typo in a doc comment changes the hash,
   and every node that hosts the type then refuses to start against the store. The only ways out are a new type name
   or a new store, and both lose data.
2. **The SQLite store takes any SQLite file.** `SqliteStore::open` adds its tables to a database that already holds the
   tables of another program. A typo in a path can change another program's database without a word.

## What Changes

- **BREAKING** The contract hash leaves out the schema keyword `description`, everywhere in the state and message
  schemas. A property that is named `description` still counts. The published contract (`Contract::to_json`,
  `GET /types/{name}`) keeps its descriptions. Every contract hash changes once, so nodes of 0.2.0 and 0.3.0 must not
  share a database.
- `SqliteStore::open` refuses a database that has no schema version (`user_version` 0) but does have tables: it answers
  `StoreError::Invalid`, names the path, and changes nothing in the file. A new or empty file is still set up.
- Version 0.3.0 (unreleased), with a changelog section.

The simplest version: the hash is taken from a copy of the contract with the descriptions removed. No setting, no new
API, no schema migration.

## Capabilities

### New Capabilities

- `sqlite-store`: how the SQLite store treats the database file it is given when it opens it.

### Modified Capabilities

- `type-registry`: adds the rule for what a contract hash covers; descriptions do not count.

## Out of scope

- The PostgreSQL store. It already works in its own database or schema; refusing foreign tables there is not asked for.
- Leaving out other keywords from the hash, such as `title` (schemars writes one only for a doc comment that starts with
  a `#` heading) or `examples`.
- Updating the stored contract when a node registers the same hash with other descriptions. The registry keeps the
  contract of the first registration, so `GET /types/{name}` keeps the descriptions it was first given (see the
  design).
- Accepting a changed contract on purpose (the other option in Chorus's request), and the other points of Chorus's
  list.

## Impact

- `crates/cicada/src/contract.rs`: how the hash is taken.
- `crates/cicada-store-sqlite/src/schema.rs` and `store.rs`: the check, and WAL mode is now set after the migration so
  that a refused file is not touched.
- A new ADR (0017), `docs/architecture.md`, README, `docs/operations.md`, `CHANGELOG.md`, workspace version 0.3.0.
- Every stored contract hash differs from the one a 0.3.0 node computes. There are no production databases (ADR 0008).
