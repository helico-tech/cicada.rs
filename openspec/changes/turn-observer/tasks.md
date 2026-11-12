# Tasks

The numbers in brackets are the decisions of `design.md`. Each group is one commit that builds and passes
`cargo test --workspace`. `control-effects` has landed when this is built (0.10.0): the schema is at version 5, and
a commit carries the control effects of its turn.

## 1. The stores

- [x] 1.1 Model and trait [8, 9]: `Message.lost_claims`, 0 for a new message; `lostClaims` in the envelope of an
      outcome, read as 0 when it is missing; the docs of `claim_due` say when it counts a lost claim, and those of
      `release`, `renew` and `commit` that they do not. Every place that builds a `Message` follows. Verify:
      `cargo build --workspace`.
- [x] 1.2 The in-memory store [9]: a claim adds 1 to the `lost_claims` of the message of the lease it replaces, when
      that message is still pending. Verify: the cases of 1.5 on the in-memory store.
- [x] 1.3 SQLite [9]: migration 6 (the column `messages.lost_claims`, 0 for every message that exists) and the same
      behaviour. The module docs say: schema version 6, not additive, and why. Verify: the cases of 1.5, and an
      upgrade test for the scenario "A database of schema version 5 is opened".
- [x] 1.4 PostgreSQL [9, 14]: migration 6 and the same behaviour; every statement that reads with `SELECT *` lists
      its columns. Verify, with `CICADA_PG_URL` set: the cases of 1.5, the upgrade test, and
      `grep -n "SELECT \*" crates/cicada/src/store/postgres` finds nothing.
- [x] 1.5 Conformance cases in a new file `lost_claims.rs` of `crates/cicada/src/tests/conformance`, one per line,
      with the helpers of the suite for a lease that runs out:
      a lease runs out and the actor is claimed again: the claim returns the message with `attempt` 2 and
      `lost_claims` 1, and after one more loss with 3 and 2;
      a lease that ran out and is not replaced counts nothing;
      a release is not counted, and neither is a release with an old epoch or a second release;
      a retry that was committed is not counted, and the count stays over it;
      a commit after the lease ran out succeeds, and the message in its outcome has 0;
      the message of a lost claim is cancelled: the next message of the actor is claimed with 0;
      the message in an outcome keeps its count.
      Verify: they pass on all three stores, on PostgreSQL with `CICADA_PG_URL` set.
- [x] 1.6 A column under an open store [14]: one test per SQL store for the two scenarios of `store-backends`: the
      store creates an actor, sends, claims, commits and lists; a column with a default is added to each table of
      the schema on another connection; the store does the same again. If SQLite fails it, its statements list their
      columns too. Verify: both pass, the one for PostgreSQL with `CICADA_PG_URL` set and failing when one statement
      is put back to `SELECT *`; the report says what SQLite needed.
- [x] 1.7 The randomised comparison [12]: it compares `lost_claims` with the rest of a message, and its lists reach
      a claim of an actor whose lease ran out. Verify: `cargo test -p cicada --lib tests::fuzz` passes; the report
      says how many generated lists reach such a claim.

## 2. The observer

- [x] 2.1 The trait and the node [1, 2, 3, 4, 5, 6]: the module `observer` with `TurnObserver`, `ObservedTurn`,
      `TurnStart` and `TurnEnd` in place of `plugin`; `ActorSystemBuilder::observe`; the node calls the observers as
      decision 3 says and catches their panics; the headers of decision 4. `Plugin`, `HookResult`, the three
      contexts, `TurnInfo`, `Attributes`, `install` and `Turn::attributes` are removed, `HandlerFailure` is private,
      and the start problem for a plugin that is installed twice goes. The test plugins of
      `crates/cicada/src/tests/runtime` and `crates/cicada-test/tests` become observers, or go with what they
      tested. Verify, on virtual time with `TestActors`: tests for the scenarios "A handler returns", "A handler
      fails and is retried", "A handler fails for good", "A message that cannot be decoded", "State that cannot be
      decoded", "A turn is cancelled", "A commit is rejected", "A lease is lost", "A node stops in the middle of a
      turn", "The actor is deleted while its turn runs", "A commit whose answer is lost", "An observer panics at
      the start", "An observer panics at the end", "An observer panics when its value is dropped", "Two observers",
      "Headers reach every send", "A send's own header wins", "Two observers add the same header", "A header that
      is too long", "A header that a message refuses", "A message that is full" and "A notification has no observer
      headers", and of `turn-management` "A handler is cancelled" with an observer.
- [x] 2.2 `cicada-otel` [7]: `ActorTelemetry` is a `TurnObserver`; the `exception` event comes from the outcome; a
      dropped value ends its span as `abandoned`. Verify: tests for the scenarios "A span per turn", "A handler
      fails and is retried" (of the telemetry), "State that cannot be decoded" (of the telemetry) and "A turn that
      commits nothing"; the tests the crate has pass.
- [x] 2.3 `cicada-extras` and the sample service [6, 13]: `CrashLoopGuard`, `CrashLoopError` and their test are
      removed; the sample service adds its telemetry with `observe` and no longer reads `CRASH_LOOP_LIMIT`. Verify:
      `cargo test -p cicada-extras` and `cargo build --workspace` pass, and
      `grep -rn "CrashLoopGuard\|CRASH_LOOP_LIMIT" crates sample-service e2e examples` finds nothing.

## 3. The claim limit

- [x] 3.1 The node [10, 11]: `NodeConfig::max_lost_claims`, `Some(5)` by default, with `Some(0)` a start problem;
      the check before any observer is called and before anything is decoded; the commit as `failed` with the error
      of decision 11; the observers told after that commit. The docs of `claim_and_abandon` say that it loses a
      claim. Verify, on virtual time with `TestActors`: tests for the scenarios "A message whose turns keep dying",
      "Fewer lost claims", "Claims that were given back do not count", "Runs that were committed do not count",
      "The retry policy is not asked", "The mailbox moves on", "A watcher is told", "An observer is told after the
      commit", "The commit of the limit does not succeed", "The message is sent again", "The default", "No limit"
      and "A limit of zero".
- [x] 3.2 The simulation [12]: its nodes run with a limit of 2, and its checks accept a failure by the limit only
      after two lost claims and without a handler run for that claim. Verify: unit tests of the checks for the
      scenarios "A scenario that reaches the limit" and "A failure by the limit that is not allowed";
      `cargo test -p cicada --lib tests::fuzz` passes, and so does one run with `PROPTEST_CASES=20000`; the report
      says how long that run took and how many generated scenarios reach the limit.
- [x] 3.3 The end-to-end tests [Impact]: where a test stops a node in good order in the middle of a turn, it also
      checks that the message has `lostClaims` 0 afterwards. Verify: the suite compiles (`cargo test -p cicada-e2e
      --no-run`); it runs in 6.2. If no test stops a node that way, say so in the report and add none.

- [x] 3.4 A claim that arrives too late [15, validation]: a node that receives a claim after its local lease deadline
      calls no observer, loads and runs nothing, and releases it; a release that fails is logged as such. Verify: the
      test for the scenario "A claim that arrives too late", which fails without the change; the conformance case of
      a release with an old epoch that leaves the newer lease as it is, on all three stores; the scenario of the
      simulation with a claim delayed by 8 s passes unchanged.
- [x] 3.5 A claim with less than a heartbeat left [15, validation, second round]: the keeper makes its first renewal at
      once when the lease it starts with has less than one heartbeat left. Verify: the test for the scenario "A claim
      that arrives shortly before its deadline", which fails without the change.
- [x] 3.6 Outages in the simulation [12, validation]: on top of its other faults, a generated scenario has up to three
      outages, each failing many renewals in a row on every node. Verify: the test of the fixed 200 scenarios asserts
      at least 10 that reach the limit and at least 10 that run a message after a lost claim below it.

## 4. HTTP, client, command line and terminal UI

- [x] 4.1 `lostClaims` in the view of a message [9]: `MessageView` and its mapping, read as 0 when it is missing;
      `ActorClient` follows; regenerate `docs/openapi.json`. Verify: tests for the scenarios "The view of a message"
      (over HTTP), "A view from an earlier node" and "The outcome keeps the count" (over HTTP); the OpenAPI test.
- [x] 4.2 The command line and the terminal UI [9]: the column `LOST` in the wide table of `cicada get messages`,
      and the line `Lost claims` in the facts of a message. Verify: a case in `tests/cli.rs` for the column, and a
      test with `TestBackend` for the line.

## 5. Docs

- [x] 5.1 `docs/architecture.md`: the term *observer* in place of *plugin*; G4 as ADR 0025 words it; the claim step,
      the row of a node that dies or loses its lease and a row for the claim limit in the turn semantics;
      `lostClaims` where a message is described; the schema paragraph (version 6, and that it is not additive), the
      crate table and how the runtime maps to tokio. `README.md`: observers in place of plugins with an example, the
      claim limit in place of `CrashLoopGuard`, the tables that name the guard or `CRASH_LOOP_LIMIT`, the known
      limitations, and the row of ADR 0025. `docs/operations.md`: "Version 0.11.0 is such a release", with every
      old node stopped first and why; the claim limit (what is a lost claim, what is not, a node that is killed
      hard, what a message that kills its node does to the messages beside it, that the count never goes back, how
      to see it, how to raise the limit or turn it off, one value on all nodes); the telemetry; that a later
      migration that only adds a column can be additive. `AGENTS.md`: the map. Verify:
      `cargo test -p cicada-examples --doc` passes, every risk of `design.md` that names the docs is in them, and
      `grep -rn -i "plugin" README.md docs/architecture.md docs/operations.md AGENTS.md` finds nothing but ADR
      titles.
- [x] 5.2 `CHANGELOG.md`, a section `0.11.0 — unreleased`, and the version `0.11.0` in `Cargo.toml`: the section
      starts with the steps of the Migration Plan of `design.md`, then the breaking changes of the proposal, then
      what changes in the telemetry and in the headers of a send. Verify: every step of the plan and every
      **BREAKING** line of the proposal is in the section.

## 6. Checks

- [x] 6.1 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, `cargo check -p cicada` with no feature and with each of `sqlite`, `postgres` and
      `testing` alone, and `openspec validate turn-observer --strict`. Verify: all pass.
- [x] 6.2 The PostgreSQL tests with `CICADA_PG_URL` set, once more against a throwaway `postgres:17.5-alpine` with
      its default 100 connections, as the database of CI has (removed afterwards), and the end-to-end suite with
      `CICADA_E2E=1 cargo test -p cicada-e2e -- --nocapture`. All need Docker. Verify: all pass; if Docker cannot be
      started, leave this task open and say so in the report.
