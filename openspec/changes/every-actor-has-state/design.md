# Design

Described with the owner on 2026-11-11 (change loop, Describe). Every decision below is made by the owner unless it
says otherwise. Reviewed twice: decisions 1 and 4 changed after the first review; the second one asked for no new
decision, and its gaps are closed in decisions 1, 8, 9 and 10, the risks and the migration plan.

## Context

What the code does today (`main` at 414a8ba, release 0.3.1):

- `ActorState` (`crates/cicada/src/message.rs`) has one blanket impl for `Serialize + DeserializeOwned + JsonSchema`
  types and one for `Stateless`. Its only real job is `STATELESS` and the three `Option`s it returns.
- State is optional in: `Claim.state`, `ActorRecord.state`, `ActorCreate.state`, `StateChange::NewState(Option<Value>)`,
  `Store::create(_, Option<Value>)`, `Contract.state_schema`, `ActorSnapshot.state`, and on the wire in
  `CreateActorRequest.state`, `ActorView.state` and `TypeView.state`. The column `actors.state` is nullable in both
  stores.
- `create_stateless` exists on `Turn`, `ActorSystem`, `ActorClient` and `TestActors`.
- `crates/cicada-fuzz` drives the `Store` trait with optional states (`src/compare.rs`, `src/faults.rs`,
  `tests/backends.rs`), and `e2e/tests/suite/cluster_api.rs` creates an actor without a state.
- `ActorSystem::create_actor` and the executor's `check_typed_effects` treat a JSON `null` state as "no state" and
  refuse a state for a stateless type. `PUT /actors/{type}/{key}` with an empty body means "no state".
- A node that does not host a type stores any state unchecked; the first turn then fails with "State cannot be
  decoded". This change keeps that; `json-schema-at-the-http-door` asks whether the door checks state too.
- In this repository every state type is a struct (a JSON object). 20 actor types use `Stateless`.
- Both stores hold their migrations as lists of SQL statements (`schema.rs`), version 1 only. No foreign keys.
- A node registers the contract of each type it hosts when it starts. It refuses to start when the registered
  contract of one of its types has another hash. A node does not open a database with a newer schema version than
  its own.
- For an actor whose type is not registered: it can be read and deleted. A create of one is refused, over HTTP as
  "no such type", and a turn that creates one ends `failed` without a retry (`check_typed_effects`). A typed send
  from a turn to an existing one still commits.
- There is no test of an upgrade: ADR 0009 removed the old ones with the old migrations, `MIGRATIONS` is private, and
  opening a store always migrates to the newest version.

Found while reading, not in the first proposal (the proposal is updated):

- **The registry blocks an in-place upgrade.** A formerly stateless type keeps its row in `actor_types` with
  `"state": null` and the old hash, so a new node that hosts the type would refuse to start. See decision 1.
- **`crates/cicada-cli` is touched.** See decision 6.
- SQLite cannot add `NOT NULL` to an existing column with `ALTER TABLE`. See decision 5.
- Contract snapshots of stateless types cannot be read by a strict contract parser. See decision 9.

No database has to survive this change: the owner says Chorus and Karakuri can start again on new ones. The in-place
path of decision 1 is kept all the same, because it costs one statement per store. It has one narrow window, which
decision 1 names.

## Goals / Non-Goals

Goals:

- One kind of actor type. No `Option` around a state anywhere: not in the store model, a commit, a contract or the
  wire model.
- The store itself refuses an instance without a state, in both backends.
- A database of schema version 1 is upgraded in place when this version opens it. The operator stops the old nodes
  and starts the new ones; nothing is reset by hand.

Non-Goals:

- A rolling upgrade with old and new nodes side by side.
- A `Default` bound on state, or a create without a state.
- Checking a state against its JSON Schema on a node that does not host the type.
- A compile-time link between a ref and its state type in `create`.

## Decisions

### 1. The upgrade works in place: migration 2 removes the registrations without a state schema

Migration 2 deletes every row of `actor_types` whose contract has no state schema (`state` absent or `null`). Those
are exactly the types that were stateless.

```sql
-- SQLite
DELETE FROM actor_types WHERE json_extract(contract, '$.state') IS NULL;
-- PostgreSQL
DELETE FROM actor_types WHERE contract->'state' IS NULL OR contract->'state' = 'null';
```

What follows:

- A node of this version that hosts such a type finds no registration and registers its new contract at start, as for
  a new type. There is no contract conflict.
- The actors of the type stay. Decision 5 gives them the state `{}`, which decodes as `NoState`. Their pending
  messages stay too.
- Between the migration and the start of a node that hosts the type, the type is not registered. Its actors can be
  read and deleted, and a message to one of them waits in its mailbox. But a create of an actor of the type is
  refused: over HTTP as "no such type", and a turn on another node that creates one ends `failed`, without a retry.
  This window exists only when nodes host different types. A node registers its types before it handles a message,
  so a node that hosts every type has no window. The upgrade note says: start the nodes that host the formerly
  stateless types first.
- If no node hosts the type any more, it stays unregistered until the operator deletes its actors.
- After the migration no stored contract lacks a state schema, so the code is strict: `Contract::from_json` refuses a
  contract whose `state` is absent or `null`.

First the owner chose "no help, as in 0.3.0" (the operator resets every stateless type before the upgrade), with a
migration step that gave leftover registrations a placeholder schema. The review showed that this worked against
itself: the reset deleted the very actors the migration converts. That path is dropped.

Not chosen: "start on a new database" as the only way (cheapest, but an old database with stateless types would give
a node that refuses to start, and unreadable registrations); rewriting the stored contracts and hashes (needs a
migration step in Rust).

`one-name-keys` changes every contract hash. Whether its migration removes registrations in the same way is for its
own design.

### 2. A state is never JSON `null`

A `()` state, a unit struct or a top-level `Option` encodes to `null`. That is refused in three places:

- `Turn::create` returns `TurnError::Invalid`, like its other checks of the initial state.
- `ActorSystem::create_actor` returns `CreateActorResult::InvalidState`; over HTTP that is `422 invalid-state`.
- A turn that ends with a `null` state ends its message as `failed`, like a state that cannot be encoded. There is no
  retry, and the stored state stays.

The stores do not check for JSON `null`: they store the value they are given, and only refuse a missing state
(decision 5). A caller of the public `Store` trait could still store a `null`; `private-store-trait`, the next
change, takes the trait out of the public API. The migration turns a stored JSON `null` into `{}`.

Not chosen: allowing `null` as a value (against ADR 0018); a `CHECK` constraint in the stores (an error path in
three stores for a case the runtime already refuses); a start-up check on the state schema (it cannot see every case,
and the first create of a `()` type already fails loudly).

### 3. `PUT /actors/{type}/{key}` needs a state

`CreateActorRequest.state` is a required field. A missing `state` or an empty body is a `400`, like every body that
cannot be decoded. `{"state": null}` is a `422` (decision 2). `ActorClient::create_actor` takes a `Value`.

Not chosen: a missing state means `{}`. That is "create without naming the state", which the proposal puts out of
scope.

### 4. `NoState` refuses unknown fields

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoState {}
```

It encodes to `{}`, and an object with any field does not decode. So a node that hosts a `NoState` type refuses a
create with a state such as `{"oops": 1}`, as it refuses a state for a stateless type today. The special refusal "is
stateless and takes no state" goes away, in `create_actor` and in `check_typed_effects`: the ordinary decode of the
state does the same job. `NoState` lives in `cicada::message` and is exported as `cicada::NoState` and in the prelude,
where `Stateless` was.

First the owner chose a plain struct, on the main session's advice that a type could then drop its state without
touching its rows. The review showed that this cannot happen: a changed state type changes the contract hash, and a
changed hash needs the type to be reset anyway. With that reason gone, the stricter struct wins.

Not chosen: a plain struct (a stray state would be accepted and stored).

### 5. The schema: migration 2 in both stores, and SQLite rebuilds the table

Migration 2 is appended in both stores. After it, `actors.state` is `NOT NULL` without a default.

- Both: every state that is SQL `NULL` or JSON `null` becomes `{}`, and every registration without a state schema is
  removed (decision 1).
- PostgreSQL: `ALTER TABLE … ALTER COLUMN state SET NOT NULL`.
- SQLite: rebuild the table. Create a new table with `state TEXT NOT NULL` and the other columns as they are, copy
  the rows, drop the old table, rename the new one to `actors`, and create the indexes `actors_due` and
  `actors_running` again. All of it runs in the one transaction of the migration.

Not chosen for SQLite: a new column with `NOT NULL DEFAULT '{}'` that replaces the old one (the default would hide a
missing state); leaving the column nullable (the two stores would differ).

Migration 2 is not additive: a node of the old version that writes a stateless actor fails on the constraint. So
every old node is stopped before the first new one starts. The module docs of both `schema.rs` files and the
Upgrading section of `docs/operations.md` say "additive" today; they get this exception.

### 6. The command line needs `--state` too

`cicada create actor <address> --state <JSON>`: `--state` is required, so a `NoState` actor is created with
`--state '{}'`. The TUI drops the "Stateful" fact of a type, and the types table drops the column that says whether a
type has state: both would always say yes. Code that handles a missing state (`Option`) in the output and the TUI
goes away with the wire model.

Not chosen: `--state` left out means `{}` (the command line would differ from the API).

### 7. ADR 0018 becomes Accepted

ADR 0018 has the status Proposed, and this change is its first step. The main session sets it to Accepted in the
commit that makes this change ready. No further ADR.

### 8. What changes where

These follow from the proposal and the decisions above.

- `cicada::message`: `ActorState` and `Stateless` are removed. `Actor::State` has the bound
  `Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static`.
- Store model: `Claim.state`, `ActorRecord.state` and `ActorCreate.state` are a `Value`; `StateChange::NewState`
  holds a `Value`; `Store::create` takes a `Value`. All three stores (SQLite, PostgreSQL, in memory) follow, and so
  does the conformance suite.
- `Contract`: the state schema is a `Value`, in `new`, `state_schema()` and the JSON form.
- `ActorSnapshot.state`, `ActorView.state` and `TypeView.state` are a `Value`. `docs/openapi.json` follows.
- `create_stateless` is removed from `Turn`, `ActorSystem`, `ActorClient` and `TestActors`. Callers write
  `create(&r, &NoState {})`.
- Every `Stateless` in the tests, the examples and the sample service becomes `NoState`.
- `crates/cicada-fuzz` and `e2e` follow the store model and the HTTP create. The end-to-end suite runs only with
  `CICADA_E2E=1` and Docker, and the PostgreSQL tests only with `CICADA_PG_URL`; the tasks run both.
- Docs: `README.md` (the turn API, the HTTP table, the test harness, the examples), `docs/architecture.md`,
  `docs/operations.md` (schema version 2, the exception to "additive") and `CHANGELOG.md` with the upgrade note at
  the top of its section.

### 9. Contract snapshots of stateless types: a changelog line, no code

Decided by the main session after the reviews.

`assert_contracts_match` (`crates/cicada-test/src/snapshots.rs`) reads each snapshot with `Contract::from_json`. A
snapshot of a stateless type has `"state": null`, which the strict parser refuses, so the check fails with "is not a
contract" and the update flag does not get past it. A missing snapshot is written and passes. So the way out needs no
code: delete the file. The three snapshots of stateless types in `sample-service/tests/contracts` are deleted and
written again, and the changelog tells hosts to do the same.

Not chosen: treating a snapshot that cannot be read as one that differs (code for a mistake that happens once).

### 10. The upgrade is tested from a version-1 database that the test builds

Decided by the main session after the second review.

Each SQL store crate gets a helper for its own tests only (`#[cfg(test)]`, no public API) that applies the
migrations up to a given version to an empty database. A test builds a version-1 database with it, writes rows with
SQL (an actor without a state, one with `null`, a registration with `"state": null`, a pending message), opens the
store, and checks the result.

- The scenarios at the level of rows (states, the constraint, the registrations, the indexes) run in both stores;
  the PostgreSQL ones run when `CICADA_PG_URL` is set.
- The scenarios that need a node (a stateless type is upgraded in place; a type that no node hosts any more) run on
  SQLite only, with a real `ActorSystem` on the migrated file. The code above the store is the same for PostgreSQL,
  and its rows after the migration are proven by the test at the level of rows.

## Risks / Trade-offs

- **A formerly stateless type is unregistered until a node that hosts it starts.** A turn on another node that
  creates an actor of the type in that window fails for good. → Only with nodes that host different types. The
  upgrade note says to start the hosts of those types first.
- **A type whose state encodes to `null` has no in-place path** (`()`, a unit struct, a top-level `Option`). The
  migration turns its stored `null` into `{}`, which the type does not decode. Its registration has a state schema,
  so it stays, and a changed state type has another hash. → The upgrade note says: change the state to a struct and
  reset the type with the old version before the upgrade (delete its actors, then `DELETE /types/{name}`), as for any
  contract change. The doc comment of `Actor::State` and the README say that a state is never `null` and point to
  `NoState`.
- **`NoState` decodes from an empty array.** serde reads a struct from a list of its fields, so `{"state": []}` is
  accepted for a `NoState` type and stored as given. → Accepted; it does no harm.
- **The SQLite migration copies every actor row once.** → Accepted; there is no production database (ADR 0008).
- **A node that does not host a type still stores an unchecked state.** → As today; left to
  `json-schema-at-the-http-door`.
- **Hosts' contract snapshots of stateless types fail once.** → Decision 9; the changelog says to delete them.

## Migration Plan

For a host (Chorus, Karakuri):

1. In the code: replace `Stateless` with `NoState` and `create_stateless(&r)` with `create(&r, &NoState {})`, and
   give every HTTP create a `state`.
2. Delete the contract snapshot files of the formerly stateless types; the next test run writes them again.
3. A type whose state encodes to `null`: change its state to a struct, and reset the type with the old version
   (delete its actors, then `DELETE /types/{name}`).
4. Stop every node of the old version.
5. Start the new version, the nodes that host the formerly stateless types first. The first node migrates the schema
   to version 2; each node registers its types.

Or start on a new database; then steps 1, 2 and 4 are all that is left.

There is no way back: a migrated database is not opened by the old version.

The version is a breaking one: 0.4.0 after 0.3.1.
