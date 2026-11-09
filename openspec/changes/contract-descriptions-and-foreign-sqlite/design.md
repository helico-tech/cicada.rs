# Design

## Context

See proposal.md, Why.

- `Contract::new` sets `hash = canonical_sha256(&self.to_json())`. The JSON is
  `{"type", "key", "state": <schema> | null, "messages": {<name>: <schema>}}`. `to_json` is also what the registry
  stores and what `GET /types/{name}` serves.
- Registering a type compares only hashes (`support::register_again`). On an equal hash the registry keeps the entry it
  already has, so the stored JSON is the one of the first registration.
- schemars puts a doc comment into the schema as `description`, on the type and on each field. A doc comment that
  starts with a `#` heading also gives a `title`. Message schemas get `title` = the message name from `schema.rs`.
- `SqliteStore::open` connects the writer with `journal_mode = WAL`, then runs `schema::migrate` in one
  `BEGIN IMMEDIATE` transaction. Setting WAL changes the file header, so today even a refused database is changed.

## Goals / Non-Goals

**Goals:**

- A doc comment change does not change a contract hash.
- A schema change that matters still does, also when it is a property named `description`.
- `SqliteStore::open` leaves a foreign database exactly as it was, byte for byte.

**Non-Goals:**

- Changing what is published or stored for a contract.
- Any change to the PostgreSQL store or the store schema.

## Decisions

### 1. Published with descriptions, hashed without

`to_json` stays as it is, with every `description`. Only the hash input drops them. Docs stay visible in
`GET /types/{name}` and in contract snapshots, and a comment change is harmless.

Other option: strip them from the published form too. It is simpler to explain (one form), but the schemas lose their
only documentation, and clients that show the schemas get less. Not chosen.

The cost: the registry keeps the JSON of the first registration. After a doc comment change, nodes start fine, but
`GET /types/{name}` keeps the old descriptions until the type's registration is removed and made again. Refreshing the
stored JSON on an equal hash would need a store change in both backends; it is left out (proposal, Out of scope).

### 2. Strip by schema keyword, not by key name

A private function walks one schema and removes the key `description` from every schema object it reaches. It goes
into a value only where JSON Schema puts a subschema:

- a map of names to schemas: `properties`, `patternProperties`, `$defs`, `definitions`, `dependentSchemas`. The keys
  are names, so a property named `description` is kept; the walk goes into each value.
- one schema or a list of schemas: `items`, `prefixItems`, `anyOf`, `oneOf`, `allOf` (each a schema or an array of
  them).
- one schema: `additionalProperties`, `additionalItems`, `unevaluatedProperties`, `unevaluatedItems`, `not`, `if`,
  `then`, `else`, `contains`, `propertyNames`.

It does not go into `const`, `enum`, `default` or `examples`: those hold data, and a key `description` in a default
value is data. A boolean schema (`true`, `false`) has nothing to strip.

The walk is applied to the state schema and to each message schema, never to the contract object itself. So a message
type that happens to be named `description` is kept.

Other option: remove every key `description` anywhere in the JSON. Shorter, but it drops a property named
`description` and keys in default values, so two different schemas could hash the same. Not chosen.

### 3. Hash input changes for everyone

`canonical_sha256` and `canonical_json` stay as they are (other code uses them for other things). `Contract::new`
hashes the stripped copy. A schema without any description gets a new hash too, because the input is built the same
way; no attempt is made to keep old hashes. This is a breaking release (0.3.0): every stored hash differs from the one
a 0.3.0 node computes, so 0.2.0 and 0.3.0 nodes must not share a database. ADR 0008 allows it: there are no production
databases.

`Contract::of` keeps using the registered hash, and `Contract::from_json` recomputes it with the new rule, so a contract
snapshot written by 0.2.0 still compares correctly under 0.3.0.

### 4. Refuse a foreign SQLite database inside the migration transaction

In `schema::apply`, when `user_version` is 0, read `sqlite_master` for a row with `type = 'table'` whose name does not
start with `sqlite_` (SQLite's own tables, such as `sqlite_sequence`). If one exists, return
`StoreError::Invalid("Database <path> is not a cicada store: it holds other tables")`. The transaction rolls back.

The check runs inside the same `BEGIN IMMEDIATE` as the migration. Two cicada processes that open one new file at once
stay safe: the second waits for the write lock and then sees `user_version` 1.

`migrate` gets the path for the message.

### 5. Set WAL after the migration

The writer connects without a journal mode and runs `PRAGMA journal_mode = WAL` only after `migrate` succeeded. A
refused database therefore keeps its own journal mode and its bytes. For a database that is already in WAL mode, the
pragma does nothing. A new file gets its tables in the default journal mode and then turns to WAL, before any reader
connects.

Other option: check for foreign tables on a separate read-only connection before the writer connects. That adds a
connection and a check outside the transaction. Not chosen.

### 6. Where the decision is written

One ADR (0017) holds both rules: the contract rule is a guarantee-level change, and the store rule is short and fits.
`docs/architecture.md` gets the contract rule in the domain language and the store rule after the store requirements.

## Risks / Trade-offs

- [Stale published descriptions after a comment change] → Written in the ADR, the README and operations guide. Remove
  and register the type again to refresh them.
- [A `# heading` doc comment still changes the hash, through `title`] → Written in the ADR and the README. Rare in
  practice; can be added later if asked.
- [A database made by another tool that has no tables, only views or triggers, is taken] → Accepted. Views and triggers
  without tables are rare, and the request names tables.
- [A text file or other non-database file] → Unchanged: SQLite fails to read it and `open` answers
  `StoreError::Unavailable`, as before.

## Migration Plan

None in code. Nodes of 0.2.0 and 0.3.0 must not share a database; the changelog says so. A 0.3.0 node refuses to
start on a store where a 0.2.0 node registered its types. Use a new store, or delete the actors of each type and remove
its registration (`DELETE /types/{name}`) before 0.3.0 nodes start.
