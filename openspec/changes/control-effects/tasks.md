# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `watches` has landed when this is built (0.9.0): a commit carries the watch calls of its
turn and the id of its event, and the stores write the notifications.

## 1. The stores

- [x] 1.1 Model and trait [5, 7, 11]: a commit carries the control effects of its turn, each an effect (pause,
      resume, purge, delete) and an address; the shared check `support::check_controls` answers
      `StoreError::Invalid` for a commit that deletes its own actor or that both pauses and resumes one actor, and
      a commit runs it first, before the store is read; the docs of the trait give the order of decision 5 and the
      causes of decision 11. Every place that builds a `CommitRequest` follows. Verify: `cargo build --workspace`.
- [x] 1.2 The in-memory store [1, 2, 3, 5, 7, 11]: the effects in the order of decision 5; a delete voids a turn
      that runs on the target; a delete of an ancestor of the committing actor rejects the commit; an effect on an
      actor that is not there does nothing; a restart of an actor that is not there removes nothing. Verify: the
      cases of 1.5 on the in-memory store.
- [x] 1.3 SQLite: the same. Verify: the cases of 1.5.
- [x] 1.4 PostgreSQL [8]: the same; the controlled actors and the watchers of each deleted actor join the one
      ordered lock of the commit, and the watches are read again after it, as `watches` does; for an address that a
      commit both deletes and creates, the store holds the lock of a row there before it removes and creates; a
      resume is announced as a resume from outside is. Verify, with `CICADA_PG_URL` set: the cases of 1.5, and the
      two cases for PostgreSQL of decision 10, each run 50 times in one test (the scenario "A restart of an actor
      that is not there races a create" is one of them).
- [x] 1.5 Conformance cases in a new file `control.rs` of `crates/cicada/src/tests/conformance`, one per line:
      a pause, a resume and a purge by a commit do what the calls from outside do, also to the committing actor;
      a delete by a commit removes the actor, its mailbox and its descendants, and voids a turn that runs on one of
      them;
      a commit that fails or is rejected applies no control effect;
      an effect on an actor that is not there does nothing, and a send to a deleted actor still rejects the commit;
      a delete of an ancestor of the committing actor rejects the commit and removes nothing;
      deletes come before creates (a restart, also as a child of the committing actor), creates before purges,
      pauses and resumes, and those before sends;
      a delete with a purge or a pause of the same actor and no create applies the delete;
      a restart of an actor that is not there creates it and writes no `Stopped`;
      the watches of the commit are set before its deletes;
      a deleted actor's watchers read `deleted`, with `by` and `causedBy`, the watchers of its descendants `parent`;
      an actor and its descendant that are both named read `deleted`; a commit that deletes a child and stops reads
      `deleted`, `parent` and `stopped`; a watcher that the commit removes and creates again has nothing;
      a commit that deletes its own actor, or pauses and resumes one actor, is invalid.
      Verify: they pass on all three stores, on PostgreSQL with `CICADA_PG_URL` set.
- [x] 1.6 The randomised comparison and the simulation [10]: the comparison builds the four effects in its commits,
      on addresses of its pool; the simulation under faults has an actor that restarts another in some of its
      turns. Verify: `cargo test -p cicada --lib tests::fuzz` passes, and so does one run with
      `PROPTEST_CASES=20000`; the report says how long that run took, and how many generated lists reach a restart.

## 2. The runtime

- [x] 2.1 The calls [5, 6]: `Turn::pause`, `resume`, `purge` and `delete` for a ref, and `pause_at`, `resume_at`,
      `purge_at` and `delete_at` for an address. A call fails with `TurnError::Invalid` and buffers nothing for a
      ref without an address, a delete of the turn's own actor, and a delete after a create of the same address.
      After a pause and a resume of one actor the last call stands; the same effect twice is buffered once.
      Verify, on virtual time with `TestActors`: tests for the scenarios "A pause and a resume", "A purge", "A
      delete", "A turn that fails", "By address", "A pause and a purge while a turn runs on the target", "A
      restart", "A parent restarts its child", "Another actor restarts a child", "A watch and a delete in one
      turn", "Emptying a mailbox and filling it", "Creating an actor that starts paused", "The target stopped
      itself", "A commit that is rejected removes nothing", "A send still needs its target", "A turn is running on
      the target", "A restart while a turn is running on the target", "The target has children", "The target is
      watched", "Deleting itself", "Deleting its parent", "Pausing itself", "An actor it did not create", "Pause and
      resume", "The same effect twice", "A delete after a create" and "A create of a descendant of a deleted
      actor", and of `watches` "An actor is deleted by a turn".
- [x] 2.2 The node [8, 13]: it wakes its loop after a commit with a control effect, and writes one `tracing` event
      at the info level for each control effect of a commit it saw succeed, with the acting actor, the effect and
      the target. Verify: tests for the scenarios "A resume wakes the node" and "A delete by a turn wakes the
      watcher"; the report quotes the log line of one delete.

## 3. Docs

- [x] 3.1 `docs/architecture.md`: G1, G2 and S7 as ADR 0024 words them; the rows on effects and on the order of a
      commit; how a turn ends when its actor is deleted by another turn; the sentence on leases, made exact [14].
      `README.md`: the eight calls, a restart by a parent as an example, what an effect on an actor that is not
      there does, and the row of ADR 0024. `docs/operations.md`: what a restart does to a handler that is running,
      and the log line of a control effect. Verify: `cargo test -p cicada-examples --doc` passes, and every risk of
      `design.md` that names the docs is in them.
- [x] 3.2 `CHANGELOG.md`, a section `0.10.0 — unreleased`, and the version `0.10.0` in `Cargo.toml`: what a turn
      can do now, that nodes before and after can share a database, and the breaking line of the proposal with
      the new wording of G1, G2 and S7. Verify: every **BREAKING** line of the proposal and the Migration Plan of
      `design.md` are in the section.

## 4. Checks

- [x] 4.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`,
      `cargo check -p cicada` with no feature and with each of `sqlite`, `postgres` and `testing` alone, and
      `openspec validate control-effects --strict`. Verify: all pass.
- [x] 4.2 The PostgreSQL tests with `CICADA_PG_URL` set, and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. Both need Docker. Verify: both pass; if Docker cannot
      be started, leave this task open and say so in the report.
