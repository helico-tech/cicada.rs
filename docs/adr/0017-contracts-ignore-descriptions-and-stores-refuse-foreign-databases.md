# 0017. Contract hashes ignore descriptions; the SQLite store refuses foreign databases

Date: 2026-11-09

## Status

Accepted

## Context

A type's contract hash is the SHA-256 of the canonical JSON of its contract, schemas included. schemars writes doc
comments into the schemas as `description`. So a reworded doc comment changes the hash, and every node that hosts the
type then refuses to start against the store. Chorus, a user of this library, hit this: the only ways out were a new
type name or a new store, and both lose data.

`SqliteStore::open` set up any SQLite file whose `user_version` was 0, also one that already held the tables of another
program. Before that it turned the file to WAL mode. A wrong path could change another program's database.

ADR 0008 allows changes to contract hashes while there is no production database. The change is
`contract-descriptions-and-foreign-sqlite` in `openspec/`.

## Decision

- **The hash leaves out descriptions.** The hash is taken from the contract JSON with every `description` keyword
  removed from the state schema and from each message schema, at any depth where a schema can sit (`properties`,
  `patternProperties`, `$defs`, `definitions`, `dependentSchemas`, `items`, `prefixItems`, `anyOf`, `oneOf`, `allOf`,
  `additionalProperties`, `additionalItems`, `unevaluatedProperties`, `unevaluatedItems`, `not`, `if`, `then`, `else`,
  `contains`, `propertyNames`). The keys of `properties` and the like are names, not keywords: a property, definition or
  message type named `description` still counts. Data in `const`, `enum`, `default` and `examples` still counts.
- **The published contract keeps them.** `Contract::to_json`, the registry and `GET /types` keep every description, so
  the docs stay visible and a comment change is harmless. Stripping them from the published form too was the other
  option; it would leave the schemas without their only documentation.
- Every contract hash changes once, also for a schema without descriptions. No attempt is made to keep the old hashes.
- **The SQLite store opens only its own databases.** When `user_version` is 0 and `sqlite_master` holds a table whose
  name does not start with `sqlite_`, `SqliteStore::open` fails with `StoreError::Invalid`: "Database `<path>` is not a
  cicada store: it holds other tables". The check runs inside the migration transaction. WAL mode is set only after the
  migration succeeded, so a refused file is left byte for byte as it was. A new or empty file is set up as before.
  The PostgreSQL store is not changed.

## Consequences

- A reworded doc comment no longer stops nodes. A real schema change still does.
- Breaking, version 0.3.0: a 0.3.0 node computes another hash than a 0.2.0 node for the same type, so nodes of the two
  versions must not share a database, and a 0.3.0 node refuses to start on a store where 0.2.0 registered its types.
- The registry keeps the JSON of the first registration when a node registers the same hash. After a doc comment
  change, `GET /types/{name}` keeps serving the old descriptions until the type's registration is removed and made
  again. Refreshing them would need a store change in both backends; it is not done.
- A doc comment that starts with a `#` heading gives a `title` in the schema, and `title` still counts. So does a
  schemars upgrade that changes the schemas in other ways.
- Contract snapshots (`cicada_test::assert_contracts_match`) compare hashes, so a doc comment change passes them; the
  snapshot file keeps the old text until it is rewritten. A snapshot written by 0.2.0 still compares correctly, since
  its hash is computed again when it is read.
- A program that pointed a SQLite store at a database with other tables now fails to open it. Such a database must get
  its own file.
