# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `private-store-trait` has landed when this is built, so both stores and their tests are
inside `crates/cicada`.

## 1. One name per ref

- [x] 1.1 The address [2, 3, 5]: `:` joins the characters that `percent.rs` leaves as they are; `ActorAddress::of`
      takes a type name and one name; `name()` gives the decoded name and replaces `key_values()`. Verify: tests for
      the scenarios "A ref with one field", "A ref without fields", "A value holds a separator", "An address of an
      old key with two parts" and "An invalid address".
- [x] 1.2 The reader [1, 5]: replace `address/params.rs` by a small reader built on serde that takes the one string
      out of a ref and puts it back; `RefBinding` holds no list of fields; `to_params` and `from_params` go. A node
      checks the ref of each type it hosts at start by what its `Deserialize` asks for; `address()` of a ref checks
      what its `Serialize` writes. Verify: tests for the scenarios "A field that serde skips", "A field that is a
      newtype of a string", "A newtype ref", "A ref with typed parts that writes itself as a string", "A ref with
      several fields", "A ref with a number", "A ref with an enum variant" and "A ref of a type the node does not
      host".
- [x] 1.3 The refs of this repository that are not one string [5]: `BatchItem` of the examples keeps its typed
      fields and names `String` as its serde form (scenario "A ref that joins two parts"); the three test refs with
      two fields and `Num` and `Worker` get one `String`. Verify: `cargo test --workspace` passes, and no ref in the
      workspace is refused at start except the ones a test means to be.
- [x] 1.4 The canonical check [5]: a node that hosts a type still decodes an address as its ref and refuses one the
      ref does not accept; the write-back-and-compare half of `canonical_problem` and its special case in `turn.rs`
      go. Verify: a test that a singleton refuses a key other than `_`, and a test that a ref with a reader of its
      own refuses a name it does not accept, on a create over HTTP (`422`).

## 2. No key fields in a contract or a view

- [x] 2.1 `Contract` [5]: no key fields in `new`, in the JSON form or in the hash; `key_fields()` and `params_of`
      go; `from_json` ignores a `key` entry. Verify: tests for the scenarios "The JSON form", "A contract snapshot
      of an earlier version" and "Changing a ref's fields" (the hash stays when the field of a ref is renamed).
- [x] 2.2 The wire model, HTTP, client and command line [5]: `TypeView.key`, `ActorView.params` and
      `ActorSnapshot.params` go; the client follows; the command line and the terminal UI lose the columns and facts
      for them. Regenerate `docs/openapi.json`. Verify: tests for the scenarios "The view of an actor" and "The view
      of a type"; the OpenAPI test passes; the `TestBackend` tests pass.

## 3. The upgrade

- [ ] 3.1 Migration 3 in both stores [4]: remove every row of `actor_types`; rewrite `%3A` to `:` in
      `actors.address`, `messages.actor_address` and `outcomes.actor_address`, only in rows that hold `%3A`. Update
      the module docs of both schema files: schema version 3, not additive. Verify: the tests of 3.2.
- [ ] 3.2 Tests of the upgrade, on databases built with the helper that applies migrations up to a version, in
      SQLite and in PostgreSQL (with `CICADA_PG_URL` set): "An old literal colon", "A sender with a literal colon",
      "Two keys that become one name" (the open fails and the database is still at version 2 with both actors), and
      of `actor-state` "Instances with and without a state" (the schema version is the one this store writes) and
      "The indexes of SQLite survive". With a real `ActorSystem` on a migrated SQLite file: "A stateless type is
      upgraded in place" (from version 1), "A type with state is upgraded in place" (from version 2), "A type that
      no node hosts any more" and "A turn creates an actor of a type that is not registered yet". The upgrade test
      of 0.4.0 that expects a registration to stay goes. Verify: these tests pass; none is skipped on PostgreSQL
      when the URL is set.

## 4. Docs

- [ ] 4.1 `README.md` (how a ref is declared, a ref with two parts, the HTTP table without `params` and `key`),
      `docs/architecture.md` (the address section and its tables) and `docs/operations.md` (schema version 3, the
      upgrade). Verify: `cargo test -p cicada-examples --doc` passes, and none of the three files says that a key
      has several fields or that `:` separates.
- [ ] 4.2 `CHANGELOG.md`: the section starts with the upgrade steps of the Migration Plan of `design.md`. It holds
      the query that finds two keys that become one name, the order to start nodes in and what is refused until
      then, that a ref that changes its shape no longer decodes from what is stored, that clients are upgraded with
      the nodes, and that a listing prefix writes `:`. Then the breaking changes of the proposal. Verify: every step
      of the plan, every risk of `design.md` that names the upgrade note, and every **BREAKING** line of the
      proposal is in the section; the query runs on a version-2 SQLite database and finds a pair.

## 5. Checks

- [ ] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
      `openspec validate one-name-keys --strict`. Verify: all pass.
- [ ] 5.2 The PostgreSQL tests with `CICADA_PG_URL` set, and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. Both need Docker. Verify: both pass; if Docker cannot
      be started, leave this task open and say so in the report.
