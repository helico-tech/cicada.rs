# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `tidy-the-core` has landed when this is built (0.7.0): an actor definition has no
`type Message` and no `UNHANDLED`. The branch `feat/child-actors` of an earlier attempt is deleted before the build
starts; build from the design.

## 1. The stores

- [x] 1.1 Model and trait [1, 2, 4]: `ActorCreate.child`, `ActorRecord.parent`, `Backend::children(parent, cursor,
      limit)`, the shared check `support::check_creates` (creates of one address in one commit must agree; a commit
      runs it first, before the store is read), and the new wording of S2 in the docs of the trait. Every place that
      builds an `ActorCreate` or an `ActorRecord` follows. Verify: `cargo build --workspace`.
- [x] 1.2 The in-memory store [2, 3]: the parent on the row, `children`, the removal of descendants in `delete` and
      `Stop` with a set of the addresses it has seen, the rejected child create. Verify: the cases of 1.5 on the
      in-memory store.
- [x] 1.3 SQLite [2, 3, 5]: migration 4 (`actors.parent`, the index `actors_parent`), the same behaviour with one
      recursive query that uses `UNION`. The module docs say: schema version 4, not additive, and why. Verify: the
      cases of 1.5; an upgrade test for the scenario "A database of schema version 3 is opened"; a test for the
      scenario "Two rows that name each other", with rows written by hand.
- [x] 1.4 PostgreSQL [2, 3, 5]: migration 4, the same behaviour, the tree locked level by level before the clock is
      read, a level without the addresses that are locked already. The module docs as for SQLite. Verify, with
      `CICADA_PG_URL` set: the cases of 1.5, the upgrade test and "Two rows that name each other".
- [x] 1.5 Conformance cases in a new file `children.rs` of `crates/cicada/src/tests/conformance`, one per line:
      a commit creates a child of the committing actor, and other creates have no parent;
      children are listed by parent in address order, with paging, direct children only;
      a delete removes every descendant, purges their mailboxes and reports the sum;
      a stop removes every descendant, the children of the same commit included, and purges a message that the
      commit sent to one of them;
      a stop of a child leaves its parent and its siblings;
      a child create of an actor that is independent, that has another parent, or that is the committing actor
      itself rejects the commit and keeps the lease;
      creates of one address that disagree are invalid, also when the epoch is not current;
      a turn that runs on a descendant gets `LeaseLost`, and its message is purged;
      a delete that races with commits that create children leaves no orphan;
      a delete that races with a send up the tree, from a child whose address sorts before its parent's, succeeds
      and leaves no actor of the tree.
      Verify: they pass on all three stores, on PostgreSQL with `CICADA_PG_URL` set.
- [x] 1.6 The randomised comparison [6]: a create in a generated commit may be a child create, `children` is an
      operation of its own, and the parent is part of the state that is compared. Verify:
      `cargo test -p cicada --lib tests::fuzz` passes, and so does one run with `PROPTEST_CASES=20000`; the report
      says how long that run took.

## 2. The runtime

- [x] 2.1 `Turn::create_child` [2, 7], with the arguments and checks of `Turn::create`; a second create of one
      address that disagrees returns `TurnError::Invalid` and buffers nothing; `ActorSnapshot.parent`;
      `ActorSystem::list_children`; the doc comments of `ActorSystem::delete_actor` and `Turn::stop` say that
      descendants go too. Verify, on virtual time with `TestActors`: tests for the scenarios "A child is created",
      "An independent actor is created", "The turn fails", "The child exists already", "The actor belongs to nobody
      or to someone else", "One turn creates the same address both ways", "An actor names itself", "An operator
      deletes a parent", "A parent stops", "A child stops", "Pending messages in the tree", "A message sent by the
      stopping turn" and "A turn runs on a descendant".

## 3. HTTP, client and command line

- [x] 3.1 `ActorView.parent`, always written; `GET /actors/{type}/{key}/children` as a read route; the description
      of the delete route; `ActorClient::children`; regenerate `docs/openapi.json`. Verify: tests for the scenarios
      "A child", "An independent actor", "Direct children only", "Paging" and "An unknown actor"; the read-router
      test; the OpenAPI test; a client test that lists the children of an actor.
- [x] 3.2 `cicada get children ADDR` with the paging flags of the other listings; `PARENT` in the wide table of an
      actor, and the parent in its JSON and YAML output; the help of `cicada delete actor` and the question the
      terminal UI asks before a delete say that children go too. Verify: a case in `tests/cli.rs` for the scenario
      "Children as a table", one for exit code 1 when the actor does not exist, and the unit test of the columns.

- [x] 3.3 The terminal UI [10]: the screen of an actor shows the fact `Parent` and a tab `children` after `mailbox`
      and `outcomes`, filled from `ActorClient::children`; Enter on a child opens its screen; a key that is still
      free on that screen opens the parent, and the help of the UI names it. Verify: tests with `TestBackend`, like
      the ones the UI has, for the scenarios "Up to the parent", "Down to a child" and "An actor on its own".

## 4. Example and docs

- [ ] 4.1 The `fanout` example creates its items with `create_child`. Verify: a test that deleting a batch removes
      its items.
- [ ] 4.2 `docs/architecture.md` (the term *parent* in the domain language, S2, what a turn's create and stop do,
      the schema), `README.md` (the turn API, the routes, the command line, the terminal UI) and `docs/operations.md`: what a delete
      removes; a large tree, the limit of `socket_timeout` on PostgreSQL and the two ways out [Risks]; "Version
      0.8.0 is such a release", with every old node stopped first and why [5]; when a host may switch a type to
      `create_child` [8]. Verify: `cargo test -p cicada-examples --doc` passes, and every risk of `design.md` that
      names the docs is in them.
- [ ] 4.3 `CHANGELOG.md`, a section `0.8.0 — unreleased`, and the version `0.8.0` in `Cargo.toml`: the section
      starts with the upgrade steps (stop every old node, then start the new ones; switch a type to `create_child`
      for new parents only), then the breaking changes of the proposal. Verify: every step of the Migration Plan of
      `design.md` and every **BREAKING** line of the proposal is in the section.

## 5. Checks

- [ ] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`,
      `cargo check -p cicada` with no feature and with each of `sqlite`, `postgres` and `testing` alone, and
      `openspec validate child-actors --strict`. Verify: all pass.
- [ ] 5.2 The PostgreSQL tests with `CICADA_PG_URL` set, and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. Both need Docker. Verify: both pass; if Docker cannot
      be started, leave this task open and say so in the report.
