# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `one-name-keys` has landed when this is built (0.6.0). The Rust blocks of `README.md` and
`docs/operations.md` are doctests, so a task that changes a public API also changes the blocks that use it.

## 1. The actor definition

- [x] 1.1 The message type once [4]: remove `Actor::Message`; the bound on `Actor::Ref` is `ActorRef`; `handle`
      takes `<Self::Ref as ActorRef>::Message`; `Turn::send_self` and `send_self_with` take the same type; the uses
      in `actor.rs` follow. Remove the line `type Message = …;` from every actor definition of the workspace and
      from the Rust blocks of `README.md`, `docs/operations.md` and `docs/architecture.md`. Verify:
      `cargo test --workspace` passes, and no `impl … Actor for` block in the workspace or in those three files
      holds `type Message`.
- [x] 1.2 No unhandled policy [4]: remove `Unhandled`, `Actor::UNHANDLED`, their exports from `lib.rs` and the
      prelude, and the drop path of the turn (`RunEnd::DecidedDrop` and what only it used). A message that cannot be
      decoded ends `failed` with the reason, without a retry, and no handler runs. `TurnInfo::unhandled()` stays;
      its doc comment, the table at the top of `plugin.rs`, the doc of `TurnOutcome::Done` and the log line of the
      executor stop naming a policy. Remove the line `const UNHANDLED …` from every definition and from the Rust
      blocks of the three files. The seven definitions that said `Drop` (`crates/cicada-axum/tests/common/mod.rs`
      twice, `crates/cicada-client/tests/common/mod.rs`, `actor_type.rs`, `unhandled.rs` and `common.rs` in
      `crates/cicada/src/tests/runtime`, and `examples/src/singleton.rs`): a test that only showed a dropped message
      goes, and one that relied on the drop expects `failed`. Verify: tests for the scenarios "An unknown type
      name", "A payload that does not fit" and "The next message"; a test that a plugin still reads the reason from
      `TurnInfo::unhandled()`; `grep -rn "UNHANDLED\|Unhandled::\|unhandled policy" crates examples sample-service
      e2e` finds nothing.

## 2. Named errors

Every error type of this group is a public struct that holds its text, as `AddressError` does: it implements
`std::error::Error`, `Display`, `Debug`, `Clone`, `PartialEq` and `Eq`, has a doc comment, and prints the text that
was returned before. No text changes.

- [x] 2.1 `RetryError`, `RetentionError` and `IdError` [1]: `RetryPolicy::new`, `Backoff::exponential`,
      `Backoff::fixed`, `Keep::keep_for`, `Keep::last`, `MessageId::new`, `NodeId::new`, and the `TryFrom` behind
      the serde forms of all of them. The callers in the workspace follow. Verify: tests for the scenarios "A retry
      policy without attempts", "A blank message id", "A retention of zero messages" and "An error read from JSON";
      the existing tests of the texts pass unchanged.
- [x] 2.2 `MessageError`, `ContractError` and `DefinitionError` [1, 4]: `message::encode`, `message::decode`,
      `FamilyInfo::of`, `schema::message_schemas`, `Contract::from_json`, `Contract::of` and `actor::contract_of`.
      `DefinitionError` holds the list of reasons, gives them to a caller, and prints them joined with "; ".
      `cicada-axum` stops putting "Payload of '…' cannot be decoded: " in front of the reason of a `422`. The
      callers in `cicada-test`, `cicada-axum`, `cicada-client` and `cicada-cli` follow. Verify: tests for the
      scenarios "A payload that does not decode", "A contract that is not an object", "A malformed actor
      definition" and, over HTTP, "A send with a payload that does not fit" (the detail holds "cannot be decoded"
      exactly once).
- [x] 2.3 `RequestError`, and the limit checks [1]: `BulkRequest::selection` and the `delivery` of a send request
      return a `RequestError`; the texts of the `400` answers that `cicada-axum` builds from them do not change.
      `runtime::limits::check`, `check_envelope` and `check_json` become `pub(crate)`; the constants of that module
      stay public. Verify: the existing tests of the request helpers and of the limits pass, and
      `grep -rnE "pub (async )?fn [^;{]*Result<[^;{]*, *(String|Vec<String>)>" crates/cicada/src` finds nothing
      outside `crates/cicada/src/tests`.

## 3. Start problems

- [x] 3.1 `StartProblem` [2]: a public enum, open to new cases (`#[non_exhaustive]`), with two cases: a contract
      conflict (the Rust type name of the actor, the type name, the registered hash, the hash of this node, the
      reason) and any other problem with its sentence. `StartError.problems` is a list of them; `StartError` stays
      `Clone` and prints what it prints today. `hash_conflict_reason` is no longer public. Export `StartProblem`
      next to `StartError`. Verify: tests for the scenarios "A contract conflict", "A store that fails while
      registering" (a closed store), "Two problems at once", "A node that is started twice" and "The printed text of
      a conflict"; a test clones a `StartError`; the existing tests of the printed text pass unchanged.

## 4. The answers of a send

- [ ] 4.1 `SendResult::NoSuchType` [3, 5]: `ActorSystem::send_message` answers it when the type of the target is not
      registered. The order of its checks does not change: the check for a known id still runs first. `cicada-axum`
      answers `404 no-such-type` with the problem a create uses. `cicada-client`, `cicada-cli` and the terminal UI
      follow where they name the answers of a send. A retry of a failed message is not changed. Regenerate
      `docs/openapi.json` if it changes. Verify: tests for the scenarios "The type is not registered", "The type is
      registered and the actor is missing", "A repeated send after the type was removed" and "A retry while the
      type is not registered"; the existing tests that expect `NoSuchInstance` for an unregistered type expect
      `NoSuchType`.

## 5. The stores and the ref reader

- [ ] 5.1 A migration that violates a constraint [8]: in both stores, a statement of a migration (the lists in the
      two schema modules) that fails with a violated constraint (PostgreSQL SQLSTATE class `23`; SQLite
      `SQLITE_CONSTRAINT` with its extended codes) fails the open with `StoreError::Invalid`, which names the
      schema version it could not reach and holds the database's message. Every other error, and every statement
      outside those lists, keeps its mapping. Update the doc comments of both `open` functions and of both schema
      modules. Verify: the upgrade test "Two keys that become one name" expects the invalid-store error on SQLite
      and on PostgreSQL (with `CICADA_PG_URL` set), and the database is still at version 2; a test for the scenario
      "A database that is locked while it is migrated" on SQLite; the conformance suite passes on all three stores.
- [ ] 5.2 A ref that writes a unit [9]: `Writer::unit` in `crates/cicada/src/address/reader.rs` asks whether the ref
      declares no field to serde, as the check for a struct without fields does. Verify: a test for the scenario "A
      ref whose two serde halves disagree" with the three ways: a `Serialize` written by hand, `into` a unit type
      with `try_from = "String"`, and `serialize_with` on a newtype; a real singleton still has the key `_`.

## 6. Docs

- [ ] 6.1 `README.md` (the definition API without the two lines; in the HTTP table the send row says `404
      no-such-type` for a type that is not registered; the table of ADRs gets the row of 0022, and the row of 0003
      no longer says that `UNHANDLED` is required; the notes on handlers), `docs/architecture.md` (the definition
      API, the load step of a turn, the term *actor type*) and `docs/operations.md` (a migration that violates a
      constraint: both stores report an invalid store and a node stops; the upgrade text of 0.6.0 follows). ADR
      0003 and ADR 0012 get a line under Status that points to ADR 0018 for `UNHANDLED`. Verify:
      `cargo test -p cicada-examples --doc` passes; none of the three files names `UNHANDLED`, `Unhandled` or
      `type Message` outside the table of ADRs.
- [ ] 6.2 `CHANGELOG.md`, a section `0.7.0 — unreleased`, and the version `0.7.0` in `Cargo.toml`: the section
      starts with the seven upgrade steps of the Migration Plan of `design.md`, then the breaking changes of the
      proposal, then what stays as it is (the text errors of `cicada-extras` and `cicada-test`). Verify: every step
      of the plan, every risk of `design.md` that names the upgrade note or the changelog, and every **BREAKING**
      line of the proposal is in the section.

## 7. Checks

- [ ] 7.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`,
      `cargo check -p cicada` with no feature and with each of `sqlite`, `postgres` and `testing` alone, and
      `openspec validate tidy-the-core --strict`. Verify: all pass.
- [ ] 7.2 The PostgreSQL tests with `CICADA_PG_URL` set, and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. Both need Docker. Verify: both pass; if Docker cannot
      be started, leave this task open and say so in the report.
