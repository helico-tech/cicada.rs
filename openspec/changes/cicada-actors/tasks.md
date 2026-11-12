# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `cicada-app` has landed when this is built (0.13.0).

## 1. The core

- [x] 1.1 The cause of a failure [1]: `FailureCause` with its eight values, public at the crate root and in kebab-case
      on the wire; the field `cause` of `Failed`. The node decides the cause where it decides that a message fails,
      by the table of decision 1, also for a handler error whose chain of causes holds a `TurnError::Store` or a
      `StoreError`, and hands it to the store with the commit; the in-memory store, SQLite and PostgreSQL write it into
      every `Failed`. `TurnOutcome` does not change. The snapshot of the schema of `Failed` is updated. Verify: tests
      for every scenario of the delta of `watches` (each cause, the store error passed on and wrapped, a refusal with a
      store error inside, a timeout after a refusal, a retry the store cannot schedule, a new state that cannot be
      stored, a panic outside the handler, an observer that is not touched), on virtual time with `TestActors`; a
      conformance case that a failed commit writes the cause it was given, on all three stores; the scenario "A
      watcher with a `Failed` of its own" still passes for a `Failed` without a cause.
- [x] 1.2 The create by address [3]: `Turn::create_raw` and `Turn::create_child_raw`, with the checks of decision 3
      (`TurnError::Invalid` for a type that is not registered and, on a hosting node, for an address or a state the
      type cannot read; `TurnError::Store` for a failed read of the registry). Verify: tests for every scenario of
      `create-by-address`, on virtual time, with a node that hosts the type and one that does not.

## 2. The ready-made actors

- [x] 2.1 The crate and a stored message [2, 11]: `cicada-actors` in the workspace, with `StoredMessage` and its
      typed helper, and `Schedule` moved from `cicada-extras` unchanged with its tests. Verify: a test that a stored
      message holds the type name and the payload the family encodes; a `compile_fail` doctest for a message of
      another family; the tests of `Schedule` pass.
- [x] 2.2 The supervisor [4, 5, 6, 7]: `cicada.supervisor` with its state, `Start`, `StartChild` (with the retry
      policy of 7 attempts, the waits doubling from one second), the restart with its calls in the order of decision
      5 and the id of its `Restarted`, the table of causes, `Stopped`, the restart limit and both kinds of `Escalate`,
      and the rule that its own messages count only from itself. Verify: tests for every scenario of `supervisor`, on
      virtual time with `TestActors`, among them a supervisor that is the child of another, a clock that stands still,
      and a type that is registered ten seconds after the start.
- [ ] 2.3 Dead letters [8]: `cicada.dead-letters` with its capacity, `Failed`, `Redrive` (once, and what it records)
      and `Discard`, ignoring its own failures. `FailureWatcher` is not moved. Verify: tests for every scenario of
      `dead-letters`, on virtual time.
- [ ] 2.4 The topic [9]: `cicada.topic` with `Subscribe`, `Unsubscribe`, `Publish`, `Deliver` (sent by the topic
      itself, with the retry policy, the correlation id of the publish, only to a current subscriber), the watch on
      itself and the drop of a subscriber that is not there, and a refused send failing for good. Verify: tests for
      every scenario of `topic`, on virtual time.
- [ ] 2.5 The wire formats [10]: contract snapshots of the three types (`assert_contracts_match`), and sample checks
      (`assert_message_sample`, `assert_state_sample`) for what the helpers and the actors write. Verify: the
      snapshots are committed and pass; the sample checks pass.

## 3. What uses the old crate

- [ ] 3.1 `cicada-extras` goes [12]: the crate leaves the workspace; the examples and the sample service use
      `cicada-actors`; the supervision example is built on dead letters with a watch on `*` and a supervisor with a
      child that fails; the sample service hosts the three types, creates `cicada.dead-letters/_` and its watch in
      `on_start`, and loses `FailureWatcher` and `FAILURE_WATCHER_INTERVAL`, also in the end-to-end compose file.
      Verify: `grep -rn "cicada-extras\|cicada_extras\|FailureWatcher\|FAILURE_WATCHER_INTERVAL" --include='*.rs'
      --include='*.toml' --include='*.yml' --include='Dockerfile' .` finds nothing outside `target` and
      `openspec/changes/archive`; the tests of the examples and of the sample service pass;
      `cargo test -p cicada-e2e --no-run` passes.
- [ ] 3.2 Failures as an outcomes query: the requirement of `outcomes-listing` that is removed has no code of its own
      left; `GET /failures` still answers `404`. Verify: tests for the two scenarios of the delta of
      `outcomes-listing`.

## 4. Docs

- [ ] 4.1 `docs/architecture.md`: the cause in a `Failed` (the table of decision 1), the two creates by address
      among the calls of a turn, and the crate table. `README.md`: the three actors with an example each, what a
      restart loses and which actors may be supervised, where to host a supervisor, the cause of a `Failed`, the
      creates by address, and the row of ADR 0028. `docs/operations.md`: the upgrade to 0.14.0 by the Migration Plan
      of `design.md`, step by step, with a type that watches and with dead letters. `AGENTS.md`: the map. Verify:
      `cargo test -p cicada-examples --doc` passes, and every risk of `design.md` that names the docs is in them.
- [ ] 4.2 `CHANGELOG.md`, a section `0.14.0 — unreleased`, and the version `0.14.0` in `Cargo.toml`: the steps of
      the Migration Plan first, then the breaking changes of the proposal. Verify: every step of the plan and every
      **BREAKING** line of the proposal is in the section.

## 5. Checks

- [ ] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, `cargo check -p cicada` with no feature and with each feature alone,
      `cargo check -p cicada-actors`, and `openspec validate cicada-actors --strict`. Verify: all pass.
- [ ] 5.2 The PostgreSQL tests with `CICADA_PG_URL` set, once more against a throwaway `postgres:17.5-alpine` with
      its default 100 connections, as the database of CI has (removed afterwards), and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. All need Docker. Verify: all pass; if Docker cannot be
      started, leave this task open and say so in the report.
