# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `child-actors` has landed when this is built (0.8.0): the schema is at version 4, and a
delete or a stop removes a tree.

## 1. The types of a watch

- [ ] 1.1 `Failed`, `Stopped` and the cause [1, 3, 7]: public types of the core with the fields of decision 7,
      exported at the crate root. `correlationId`, `from` and `by` are always written, `null` when there is none.
      The cause is written into the schema of `Stopped` in place. Verify: a snapshot test pins both JSON Schemas as
      a contract holds them, and a test reads back what it wrote for each, with and without the optional values.
- [ ] 1.2 `WatchTarget` [2, 9]: one actor, the types under a prefix, or every type; its text form (`type/key`, a
      type name, `*`) with a reader that refuses anything else; the rule which targets match the failure of an actor
      and which its removal. Verify: unit tests for the scenarios "A prefix at a dot", "Every type" and "A text that
      is no target", and for the rule that a removal matches the address only.
- [ ] 1.3 The check of a watcher [1, 5]: a contract may watch when it holds `Failed` and `Stopped` with the schemas
      of the core's two types, compared as a contract hash compares schemas. Verify: unit tests with a family that
      declares both, one that lacks `Stopped`, and one with a `Failed` of its own.

## 2. The stores

- [ ] 2.1 Model and trait [7, 8, 9, 10]: `Message.notification`; the trait sets, removes and lists watches; a
      commit carries the watch calls of its turn in their order, and the id of its event; `delete` takes the id of
      its event; the docs of the trait say what a failed commit, a stop and a delete write. Every place that builds
      a `Message`, a `CommitRequest` or calls `delete` follows. Verify: `cargo build --workspace`.
- [ ] 2.2 The in-memory store [7, 8, 9]. Verify: the cases of 2.5 on the in-memory store.
- [ ] 2.3 SQLite [7, 8, 9, 10]: migration 5 (the table `watches` with its index, the column
      `messages.notification`, the mark in the envelope of an outcome). The module docs say: schema version 5, not
      additive, and why. Verify: the cases of 2.5; an upgrade test for the scenario "A database of schema version 4
      is opened"; a test for the scenario "A watcher that is gone", with the row removed by hand.
- [ ] 2.4 PostgreSQL [7, 8, 9, 10]: migration 5 and the same behaviour, with the locks of decision 8: the watchers
      of the one actor of a failure, a stop or a delete are read first, locked with the other rows in address order
      and read again; a watcher that exists and is not locked runs the transaction again; one that does not exist is
      skipped; all of it before the clock is read; the watchers of descendants after the tree. A notification is
      announced as a send is. Verify, with `CICADA_PG_URL` set: the cases of 2.5, the upgrade test, "A watcher that
      is gone", and the two cases for the locks of decision 12, each run 50 times in one test.
- [ ] 2.5 Conformance cases in a new file `watches.rs` of `crates/cicada/src/tests/conformance`, one per line:
      a commit sets and removes watches of its actor, in the order of the calls, and a failed commit sets none;
      watches are set, removed and listed from outside, with both filters and paging, and a second set changes
      nothing;
      a failed commit writes one `Failed` per watcher, with every field of decision 7, the sender, no `causedBy` and
      the mark;
      a `Failed` is kept by the `failed` rule of the failed message's retention, for both of its outcomes;
      a rescheduled, a done and a cancelled commit write nothing;
      a type prefix matches at a dot, `*` matches every type, and neither is told of a removal;
      a stop and a delete write one `Stopped` per watcher of each removed actor, with the cause, `by` and, for a
      stop, the `causedBy`;
      a watcher that is removed in the same transaction gets nothing, and its watches are gone;
      removing an actor removes the watches it holds, and the watches on it stay over a delete and a new create;
      a notification that fails writes nothing, and a message of the type `Failed` that a sender wrote is not
      marked;
      the notifications of one event have the ids `<event>.0`, `<event>.1`, … in the order of the actor's and then
      the watcher's address, one per watcher and actor;
      an actor that watches itself is told of its failures and gets nothing when it stops.
      Verify: they pass on all three stores, on PostgreSQL with `CICADA_PG_URL` set.
- [ ] 2.6 The randomised comparison and the simulation [12]: the comparison sets and removes watches, in a commit and
      from outside, lists them, and compares them and the notifications with their ids; the simulation under faults
      gets a watcher that is never removed, a check that every `failed` outcome of a watched actor has exactly one
      `Failed` for it, and counts a notification as accepted. Verify: `cargo test -p cicada --lib tests::fuzz`
      passes, and so does one run with `PROPTEST_CASES=20000`; the report says how long that run took.
- [ ] 2.7 The steady conformance case [14]: the last claim of
      `s7_release_expiry_and_lost_commits_keep_the_message_pending` takes a lease that is long on every fixture, and
      so does every other case that claims with the default lease on a real-time fixture and then needs the lease to
      hold. Verify: the report lists the cases that were changed; the PostgreSQL conformance run passes five times
      in a row.

## 3. The runtime

- [ ] 3.1 In a turn [8, 9]: `Turn::watch` and `Turn::unwatch`, buffered and applied by the commit before the other
      effects; `watch` fails at the call when the target is no target or the actor's own type may not watch. The
      runtime draws the id of the event for every commit and every delete, and wakes its loop after a commit that
      ended a message as `failed` or stopped its actor, and after a delete. Verify, on virtual time with
      `TestActors`: tests for the scenarios "A turn sets a watch", "A turn removes a watch", "A turn removes a watch
      it does not hold", "A turn sets and removes one watch", "A turn that fails sets no watch", "A turn of an actor
      without the two messages", "A message fails for good", "An attempt that is retried", "A message that
      succeeds", "An actor stops itself", "A child is removed with a parent that stops", "Two actors that watch each
      other", "A target that does not exist yet", "The target is deleted and created again" and "A delete wakes the
      watcher".
- [ ] 3.2 From outside [6, 9]: `ActorSystem::watch`, `unwatch` and `watches`, with the refusals of decision 6. A
      retry of a failed notification is an ordinary message. Verify: tests for the scenarios "A watcher that does
      not exist", "A watcher whose type is not registered", "A watcher whose type lacks a message", "A watcher with
      a `Failed` of its own", "A failed notification is sent again from outside", "A sender cannot mark a message"
      and "A retry by filter after a watcher was told".

## 4. HTTP, client, command line and terminal UI

- [ ] 4.1 The routes of `/watches` [4, 9]: `GET` as a read route, `PUT` and `DELETE` as write routes; the problem
      types `no-such-watch` and `not-a-watcher`; `notification` in the view of a message; `ActorClient` follows;
      regenerate `docs/openapi.json`. Verify: tests for the scenarios "Setting and removing over HTTP", "Removing a
      watch that does not exist", "An actor is deleted" (over HTTP), "The watches of one watcher", "The watches of
      one target" and "Both filters, and a bad one"; the read-router test; the OpenAPI test; a client test.
- [ ] 4.2 The command line [9]: `cicada get watches` with `--watcher`, `--target` and the paging flags,
      `cicada create watch --target … --watcher …` and `cicada delete watch`; the output of a message shows whether
      it is a notification. Verify: a case in `tests/cli.rs` for the scenario "The command line", and one for the
      exit code of a delete of a watch that does not exist.
- [ ] 4.3 The terminal UI [13]: the listing `:watches`, with Enter on a row opening the watcher. Verify: a test with
      `TestBackend` for the scenario "The view of watches".

## 5. Docs

- [ ] 5.1 `docs/architecture.md` (G2, G4, S2, the rows on effects and on the commit, the table of how a turn ends,
      the terms *watch*, *watcher* and *notification*, the schema), `README.md` (the turn API, the two message types
      with an example of a watcher, the routes, the command line, the terminal UI, the row of ADR 0023) and
      `docs/operations.md`: "Version 0.9.0 is such a release", with every old node stopped first and why [10]; a
      type that starts to watch changes its contract; what a watcher of every type costs; the watcher of a removed
      root and its lease [Risks]. Verify: `cargo test -p cicada-examples --doc` passes, and every risk of
      `design.md` that names the docs is in them.
- [ ] 5.2 `CHANGELOG.md`, a section `0.9.0 — unreleased`, and the version `0.9.0` in `Cargo.toml`: the section
      starts with the upgrade steps of the Migration Plan of `design.md`, then the breaking changes of the proposal,
      then the fix of the conformance case. Verify: every step of the plan and every **BREAKING** line of the
      proposal is in the section.

## 6. Checks

- [ ] 6.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`,
      `cargo check -p cicada` with no feature and with each of `sqlite`, `postgres` and `testing` alone, and
      `openspec validate watches --strict`. Verify: all pass.
- [ ] 6.2 The PostgreSQL tests with `CICADA_PG_URL` set, and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. Both need Docker. Verify: both pass; if Docker cannot
      be started, leave this task open and say so in the report.
