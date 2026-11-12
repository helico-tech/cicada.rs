# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `turn-observer` has landed when this is built (0.11.0).

## 1. The core

- [x] 1.1 The checked calls [1, 2]: `SchemaCheck`, `ActorSystem::send_message_checked` and
      `ActorSystem::create_actor_checked`; `send_message` and `create_actor` are these with a check that accepts
      everything. The check gets the schema of the message type, or the state schema, and runs where decision 1
      says. A refusal is `SendResult::InvalidPayload` or `CreateActorResult::InvalidState` with the check's reason,
      and nothing is stored. The docs of `schema` no longer say that nothing validates. Verify, with `TestActors`:
      tests that the check gets the right schema and value; that its refusal stores nothing; that it is not called
      for a repeated id, a type that is not registered, a message type the contract does not have, or a `null`
      state; and that on a hosting node it runs before the decoder.

## 2. The door

- [x] 2.1 The validator [3, 4, 6, 8]: the dependency `boon` in `cicada-axum` only, and one private function that
      checks a value against a schema: draft 2020-12, a loader that refuses every address, no format asserted, the
      detail of decision 8. A schema that cannot be used is not enforced, and warned about once for each such
      schema. Verify: unit tests for the scenarios "Two places that do not match", "A field inside an optional
      part", "More than five places", "A schema that refers to a file" (with a file that exists, and the test shows
      it is not read) and "A schema with a pattern the validator does not know"; a test that a bad `date-time`
      passes; and `cargo tree -p cicada -e normal` does not name `boon`.
- [x] 2.2 The handlers [1, 5, 7]: a send and a create call the checked calls with the validator. Both refusals answer
      `422` with the problem types they have; the detail says which check refused. Verify, in the HTTP tests of
      `cicada-axum`, with a node that hosts the type and one that does not on one store: tests for the scenarios "A
      node that does not host the type", "A node that hosts the type", "A payload that matches", "A field that the
      type does not know", "A required field is missing", "A message type the contract does not have"; of the
      create "A node that does not host the type", "A state that matches", "The actor exists"; "A repeat of a
      message that does not match", "A repeat of a message whose outcome is hidden", "A send without an id"; "A
      payload that the schema allows and the type cannot decode", "The same payload on a node that does not host
      the type", "A state that the schema allows and the type cannot decode"; "A date that is not a date", "A state
      schema that cannot be used"; and of `undecodable-messages` "A send with a payload that does not fit".
- [x] 2.3 The sample checks [9]: `assert_message_sample` and `assert_state_sample`, public in `cicada-axum`. Verify:
      tests for the scenarios "A sample that passes both", "A sample that only the decoder accepts", "A sample that
      only the schema accepts", "A state sample" and "A schema that cannot be used".
- [x] 2.4 The repository's own types [9]: sample checks for the types of the examples, of `cicada-extras` and of the
      sample service, and for `Failed` and `Stopped` in a watcher's family: what each type serialises passes both.
      Where a schema refuses what its type writes, the schema is made right. Verify: the checks pass; the report
      lists every type whose schema changed, and the changelog names each as a contract change.

## 3. Client, command line and end-to-end tests

- [ ] 3.1 `cicada-client` and `cicada-cli` [7]: the refusal of a node that does not host the type reaches the caller
      as the problem `invalid-payload` or `invalid-state` with its detail. Verify: a client test against such a
      node, and a case in `tests/cli.rs` that prints the detail.
- [ ] 3.2 The end-to-end suite [Context]: `e2e/tests/suite/cluster_api.rs` expects `422` from the node that only
      sends, for the bad payload it accepts today. Verify: `cargo test -p cicada-e2e --no-run`; the suite runs in
      5.2.

## 4. Docs

- [ ] 4.1 `docs/architecture.md`: the row on validation, and the HTTP contract where it says payloads and states are
      read leniently. `README.md`: the rows of the two routes, what the door checks and what it does not (formats, a
      schema it cannot use, JSON that code sends), the sample checks with an example, and the row of ADR 0026.
      `docs/operations.md`: the upgrade to 0.12.0 (run the sample checks first; a schema fix is a contract change,
      with a pointer to "Changing a contract") and the warning for a schema that cannot be used. The docs of
      `api::problem_types` and the route table of `cicada-axum`. Regenerate `docs/openapi.json`. Verify:
      `cargo test -p cicada-examples --doc` passes, the OpenAPI test passes, and every risk of `design.md` that
      names the docs is in them.
- [ ] 4.2 `CHANGELOG.md`, a section `0.12.0 — unreleased`, and the version `0.12.0` in `Cargo.toml`: the section
      starts with the steps of the Migration Plan of `design.md`, then the breaking change of the proposal, then
      that the status stays `422` where ADR 0018 said `400`. Verify: every step of the plan and every **BREAKING**
      line of the proposal is in the section.

## 5. Checks

- [ ] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, `cargo check -p cicada` with no feature and with each of `sqlite`, `postgres` and
      `testing` alone, and `openspec validate json-schema-at-the-http-door --strict`. Verify: all pass.
- [ ] 5.2 The PostgreSQL tests with `CICADA_PG_URL` set, once more against a throwaway `postgres:17.5-alpine` with
      its default 100 connections, as the database of CI has (removed afterwards), and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. All need Docker. Verify: all pass; if Docker cannot be
      started, leave this task open and say so in the report.
