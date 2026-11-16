# Tasks

- [x] 1. Monitor contract and source. Add node_down.rs exports for singleton ref, messages, validated state,
  NodeDown payload, NodeSnapshotSource and generic actor. Implement the ActorSystem reader without start/heartbeat,
  five-second timeout and full-snapshot validation/1000-row cap with bounded lastProblem. Add the minimal Tokio time
  dependency. Verify state/schema/decoder boundaries and fake-source valid/error/timeout/regression/cap cases using
  paused time; check an actual unstarted reader returns rows without registering itself.

- [x] 2. Durable polling and deduplication. Implement guarded Poll, Rearm, ten-second scheduling, dead-watermark
  transitions and atomic Topic publishes with normal retry/failure rules. Add virtual-time tests for the exact
  threshold, first dead sample, repeated/flapping/new-row episodes, absence/pruning, early/stale/copied polls,
  preserved history on source failure, no recovery notifications and missing Topic rollback/repair. Verify targeted
  cicada-actors tests, including competing systems and lost-commit replay with one committed publication.

- [x] 3. Contract and checked example. Extend wire_formats.rs with monitor state/message references and NodeDown
  receiver samples; generate docs/reference/contracts/cicada.node-down.json with the normal update variable. Add
  examples/src/node_down.rs and tests: explicit Topic/subscriptions, bounded sink with NodeDown/Failed/Stopped,
  monitor failure watch, lastProblem inspection and repair/Rearm. Verify wire, example and contract tests and ensure
  existing actor contracts and portable core fixtures have not changed.

- [ ] 4. Routes and docs integration. Export the example and add both skills' route/coverage entries for actors
  guide Node-down alerts, its generated reference and example. Write docs-handoff.md for the guide roster, operations
  startup/flapping/retention/row-cap/read-cost limits, failure watching versus lastProblem, core linkage and architecture
  observability wording. Name any G2 proving tests for the docs owner. Integrate exact owner-prepared blocking prose
  before landing; verify docs/doctests/skill checks without route skips or new claim authority.

- [x] 5. ADR and release. Reconcile ADR 0054/index and handoff with final tests and limitations. Apply the normal
  next patch release pins/changelog, regenerating versioned references where required. Re-run affected actor/wire/
  example/docs/skill checks after integration. No new backend operation/migration, HTTP source adapter, heartbeat
  registration, recovery alert or automatic sample-service monitor activation belongs to this change.

- [ ] 6. Finishing. Run `openspec validate node-down-alert --strict`. Commit implementation and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Its full checks include
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill checks and Docker E2E. Record verification/limits. The builder does not push,
  tag, release or archive; publication remains with the build loop.
