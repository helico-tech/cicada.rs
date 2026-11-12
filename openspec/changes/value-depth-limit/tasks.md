# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `schema-reference-names` has landed when this is built (0.15.0). Values from before are
written as the section "Tests of values from before" of `design.md` says. Runtime tests run on virtual time with
`#[tokio::test(start_paused = true)]` and `TestActors` where they need no HTTP server.

## 1. The limit

- [ ] 1.1 The check [1, 2]: `runtime::limits::MAX_DEPTH = 64`, and `check_json` finds a NUL character and measures the
      depth in one walk that stops at the first level beyond the limit, with the error `The <what> is nested deeper
      than 64 levels`. Verify: unit tests for 64 levels accepted and 65 refused (arrays, objects, mixed), a scalar,
      a NUL character at depth, and the scenario "A very deep value" (built without a walk that calls itself, leaked).
- [ ] 1.2 Where it is refused [2]: a send and a create from outside a turn (`OpError::Invalid`), in a turn by a typed
      ref and as raw JSON (`TurnError::Invalid`), and the new state of a turn (`failed`, cause `handler`); the error
      of a new state that a check refuses (`null`, NUL, too deep) starts with `State cannot be stored:`. Verify: the
      scenarios of the requirement "A payload and a state are at most 64 levels deep" except the HTTP ones, and "A deep
      value sent from code".
- [ ] 1.3 The retry [2]: `RetryResult::TooDeep` with its detail, answered `409 message-not-retryable` by
      `cicada-axum`; `bulk_retry` skips it. Verify: the scenarios "A retry of a deep message from before" and "A bulk
      retry" at `ActorSystem` in `crates/cicada/src/tests`, and "A retried notification at the limit" over HTTP; the
      OpenAPI document, regenerated if it changes.
- [ ] 1.4 Values from before [3]: nothing changes when a value is read. Verify: the scenarios of the requirement
      "Values stored before keep working", in `crates/cicada/src/tests` through the private backend.

## 2. HTTP

- [ ] 2.1 The door [2]: a send and a create over HTTP of 65 levels are `400` on a node that hosts the type and on one
      that does not; `deep_values.rs` checks 64 levels where it checked 126, `memory.rs` 64 where it checked 100, and
      the schema check keeps its own thread. Verify: the scenarios "A send over HTTP", "A create over HTTP", "A value
      at the limit", the ADDED and MODIFIED requirements of the delta `http-schema-check`, and "A repeat beyond the
      depth limit".
- [ ] 2.2 Views and decoding [1]: Verify: the scenarios "A message at the limit", "The deepest view" and "An actor at
      the limit" read with `cicada-client`, and "Three shapes at the limit" in a debug build on threads with their
      default stack.

## 3. The ready-made actors

- [ ] 3.1 Dead letters, the topic and the supervisor do not change [4]. Verify: the scenarios "The deepest payload a
      letter keeps", "A payload too deep for a letter" and "A deep letter from before" (raw SQL into a SQLite store)
      in the tests of `cicada-actors`; their contract snapshots do not change.

## 4. Docs

- [ ] 4.1 The core guide (the limit, where it is refused, the untagged enum that holds itself, the `Value` too deep
      for its own drop, the decoder of a type with large fields in a debug build), the HTTP guide (the `400`, the
      `409` of a retry, a view at most 69 levels deep, the default of 64 of .NET parsers), the guide of the ready-made
      actors (61, 62, 61 and 60, the ways out of decision 3), `docs/architecture.md` (the rows of a new state that
      cannot be stored in the table of outcomes and the table of causes, and in the HTTP contract that the limits come
      first, before the repeated id and the schema check), `docs/operations.md` (memory per body; the upgrade to
      0.16.0 by the Migration Plan and decision 3), and ADR 0029 as built. Verify: `cargo test -p cicada-examples
      --doc` and the link check pass, and every risk of `design.md` that names the docs is in them.
- [ ] 4.2 `CHANGELOG.md`, a section `0.16.0 — unreleased`, and the version `0.16.0` in `Cargo.toml`: the steps of the
      Migration Plan first, then each **BREAKING** line of the proposal, the new `RetryResult::TooDeep`, and what Chorus
      must check. Verify: every step of the plan and every **BREAKING** line of the proposal is in the section.

## 5. Checks

- [ ] 5.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, `cargo check -p cicada` with no feature and with each feature alone,
      `openspec validate value-depth-limit --strict` and `openspec validate --specs --strict`. Verify: all pass.
- [ ] 5.2 The PostgreSQL tests with `CICADA_PG_URL` set, once more against a throwaway `postgres:17.5-alpine` with
      its default 100 connections (removed afterwards), and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. All need Docker. Verify: all pass; if Docker cannot be
      started, leave this task open and say so in the report.
