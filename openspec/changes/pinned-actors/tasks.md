# Tasks

- [x] 1. Core creation and execution identity. Add CreateOptions, existing-method defaults and options variants for
  typed/raw/child/checked creates. Carry pins through effects/reports and inject actual owner into Turn::node_id for
  nodes and workers. Validate pin limits on each boundary; retain existing-actor no-op and foreign-child refusal,
  rejecting conflicting buffered pins before side effects. Verify focused core/turn/completion tests, including
  rollback, repeated creates, invalid data and actual owner on paused-time TestActors runtime/worker cases.

- [x] 2. Store placement. Append migration 8 plus the partial placement/type/due index in both SQL stores; extend
  memory, records and private backend/handle methods. Filter claim candidates before limit/mutation and pass owner
  to next_due_at and dispatcher sleep. Extend conformance, upgrade and fuzz checks for foreign/matching/unpinned
  work, counter preservation, pause, expired lease/reclaim, child independence, delete/recreate and preserved old
  rows. Verify core conformance/upgrade tests with SQLite and PostgreSQL plus targeted fuzz; keep old migrations.

- [x] 3. HTTP, clients and portable fixtures. Add optional pinnedTo to create and returned-create wire forms, omit
  absent request fields, default old response fields and expose snapshot placement. Update routers/client helpers,
  conversions and OpenAPI. Require valid pinned-create lease fixtures, invalid-pin conversion cases and pinned
  coverage while keeping old cases byte-identical. Verify core API/fixtures, axum and client tests including owner B
  leasing through server A, no node-row requirement, invalid fields and read-after-create; regenerate OpenAPI only.

- [x] 4. Operator visibility. Add --pinned-to to actor create in local/remote command paths, show placement in wide
  state lists, describe/JSON and existing TUI actor detail. Preserve mailbox rows, pagination and operator controls.
  Verify CLI parse/error, command/output, local/HTTP parity and relevant TUI tests. Regenerate CLI reference with its
  existing update variable. Do not add list filters, node-detail actor fanout or ready-made actor creation options.

- [x] 5. ADR, handoff and release. Reconcile ADR 0052/index and docs-handoff.md for architecture/S8, migration/drain
  procedure, stable unique ids, create/child/worker APIs, pending-work diagnosis, wire fields and Rust struct-literal
  changes. Request S8 proving-test rows and exact owner fixes to any affected guide/README snippets; integrate before
  landing. Include normal patch release pins/changelog and verify docs/doctest/skill checks. The process example and
  alarms remain successors; no new production process management belongs here.

- [ ] 6. Finishing. Run `openspec validate pinned-actors --strict`. Commit the implementation and run
  `python3 scripts/gate.py run --e2e`, then report `python3 scripts/gate.py status`. The gate includes
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` with PostgreSQL,
  feature builds, script/docs/skill/fixture checks and Docker E2E. Record compatibility checks and limits. The builder
  does not push, tag, release or archive; publication remains with the build loop.
