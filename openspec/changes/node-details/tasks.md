# Tasks

- [x] 1. Core details and validation. Add NodeConfig.label (default None), its validation and optional fields to
  NodeInfo/NodeView's core mapping inputs and private NodeRecord. Capture normalized whoami hostname once after
  initialization and before background work, plus core crate version and label; add the direct dependency and
  private NodeDetails argument to heartbeat_node. Update all backend/fault/fuzz signatures and constructors with
  task 2 so each commit compiles; do not expose NodeRecord or NodeDetails. Unit-test normalization failures,
  Unicode count boundaries, NUL, blank and preserved spaces. Virtual-time startup/operator tests await a heartbeat
  and check label/version, send-only rows, unchanged lifecycle and no lookup on initialize-only execution.
  Verify: targeted `cargo test -p cicada --lib tests::runtime::startup` and runtime operator tests, plus hostname
  normalization unit tests. No test relies on a hardcoded hostname or mutates the environment.

- [x] 2. Stores and additive migration. Append migration 7 in SQLite and PostgreSQL; update explicit node projections,
  inserts and upserts, memory behavior and schema rustdoc. Preserve timestamps and replace details including None.
  Add conformance cases for Unicode round-trip and overwrite/clearing on all three backends. Add schema-6 upgrade
  cases for old rows, a pre-migration connection's prepared read/heartbeat after migration, old four-column
  insert/upsert, and existing newer-schema refusal. Update wrappers/fuzz setup as needed; never rewrite migrations.
  Verify: `cargo test -p cicada --features sqlite,testing tests::conformance`, SQLite upgrade tests, and the
  PostgreSQL conformance/upgrade tests with CICADA_PG_URL set. Record live migration evidence; an unset URL is a skip.

- [x] 3. HTTP and clients. Add optional hostname/cicadaVersion/label wire fields with defaults and omission when
  absent, map both node endpoints, and regenerate OpenAPI. Test fields on GET /nodes and GET /nodes/{node}, old
  responses with fields absent, null values, and unchanged flattened detail. Verify targeted node tests in
  cicada-axum and cicada-client, then `CICADA_UPDATE_OPENAPI=true cargo test -p cicada-axum --test openapi` and the
  same OpenAPI check without the variable. No endpoint or client method is added.

- [x] 4. App setting and displays. Add --node-label/CICADA_NODE_LABEL using raw variable text and the specified
  precedence; reject invalid explicit labels with usage errors. Refuse the flag for HTTP workers, while ignoring
  their variable/host label. Test flag/env/host precedence, empty versus whitespace-only variables, preserved outer
  spaces, invalid values and malformed unused HTTP-mode labels. Add HOSTNAME and wide VERSION/LABEL through shared
  Columns output, escaping control characters only for table/TUI metadata cells. Test old missing fields, Unicode,
  newline/escape characters, unchanged JSON values, and a TUI node view including navigation. Regenerate CLI.
  Verify: `cargo test -p cicada-app --features tui,sqlite` and the CLI reference check with CICADA_UPDATE_CLI=true,
  then without it. Preserve other fields' rendering and existing command defaults.

- [x] 5. ADR, breaking release and handoff. Reconcile ADR 0048/index with the final implementation. Apply 0.19.0
  (or the next breaking minor if main advanced) through the normal Cargo/lock/skill/OpenAPI version steps and add
  the changelog. Write docs-handoff.md with operations' metadata/stable-id limits, App setting and HTTP exclusions,
  architecture's schema/node changes, upgrading's public-field break and old-binary reopen/rollback restriction,
  optional core guidance and release pins. State duplicate-id and mixed-version staleness; do not promise identity.
  No handwritten prose edits, no new guide route or G/S row. Verify examples doctests, docs and skill tests; report
  any unexpected protected-prose failure as blocked rather than editing it.

- [ ] 6. Finishing. Run `openspec validate node-details --strict`. Commit the final tree and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. Its full policy includes
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  independent feature builds, script/docs/skill tests and Docker E2E. Record tested hostname platforms and migration
  evidence, preserving actor contract snapshots without regeneration. Do not install hooks, push, tag or release
  from the builder; publication belongs to the owner-approved build loop.
