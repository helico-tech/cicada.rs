## Implementation

- [x] 1. Reconcile the landed core completion/Worker names, then add the documented request/result/report/effect
  views and strict TurnMessageView in cicada::api, with conversions preserving every core field, delivery kind,
  null and effect order. Keep existing management views unchanged. Add api unit tests for every result/report,
  all execution fields and policies, omitted versus null required fields, unknown envelope fields and variants,
  i64 extremes/invalid numeric forms, nanosecond/overflow durations and canonical addresses. Explicitly assert the
  generated schema required lists for nullable-but-required fields match decoder behavior; ordinary Option derives
  alone are insufficient. Run targeted API tests and crate doctests.
- [x] 2. Add full-router lease handlers, routes, operationIds and OpenAPI entries. Share bounded byte reading and
  add the route-specific duplicate-rejecting parser, leaving old routes unchanged. Cover escaped duplicate keys at
  every depth, empty/non-object/trailing bodies, missing/unknown/mistyped fields, size limits with/without length,
  parser depth versus core storage-depth validation and all operation/error alternatives. In
  crates/cicada-axum/tests/leases.rs, use virtual time and router oneshot requests for handler-free/unstarted/peer/
  stopped/closed systems, hash mismatch, release idempotence, fencing and read-router exclusion. Assert no server
  completion observer but preserved claim-limit observers. Run affected axum tests, regenerate docs/openapi.json
  with CICADA_UPDATE_OPENAPI=true and check required fields and route access in OpenAPI tests.
- [ ] 3. Implement TurnSource for ActorClient using raw calls, the explicit status/result conversion and positive
  turn_timeout (default 10s) through body reading; add async-trait from the workspace. Preserve management retries
  and supplied HTTP client settings. Add registration's recognized conflict/invalid-contract alternatives and one
  diagnostic GET, recomputing requested_hash from schemas and retaining served definitions. Test every status class,
  misleading Problem.status, malformed success, nullable field omission, wrong/unknown policy, URL/encoding errors,
  shorter HTTP timeout, pending body timeout and request counts with send_attempts > 1 in client turn_source tests.
  Test a vanished registration conflict and stale cached Contract hash. Use controlled barriers and virtual time for
  timeout assertions; run targeted client tests.
- [ ] 4. Add Worker/ActorClient integration and fault tests with unchanged Rust actors and two HTTP servers sharing
  a store. Lease on A, renew/complete on B, stop A mid-turn, and cover operator cancellation through renewal. Drop a
  completion response after application and prove one effect set without handler rerun; cover Rejected followed by
  an uncertain failed-report commit, with no restoration of original effects or release. Cover 413 abandonment and
  no hidden fallback, peer TTLs and unknown responses. Use start_paused with controlled I/O barriers/clock advancement
  for runtime assertions, following AGENTS.md; real-socket framing smoke tests need no timer-sensitive behavior.
  Run the targeted integration tests, including SQLite/PostgreSQL peer cases where available. Add a compiled
  no_run example in examples/ of a database-free worker using an explicitly configured HTTP client.
- [ ] 5. Update ADR 0040/index as needed, architecture's HTTP contract, HTTP guide, operations and crate route table.
  Document full envelopes, exact integer parsing, nanosecond TTLs, raw canonical body addresses, trusted auth covering
  /leases and its children plus type registration, redirects/custom retry restrictions, effective timeouts, whole-report
  body/proxy limits, deep legacy states, claim-limit recovery and uncertain completion. Link the example, and relevant
  new fencing/atomicity/effects-once tests in docs/guarantees.md. Update changelog/version; run examples doctests and
  docs checks. No CLI settings, receipt service, worker credentials or database migration belong to this change.

## Finishing checks

- [ ] 6. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate remote-turns-over-http --strict`. Include PostgreSQL with CICADA_PG_URL where available and
  record unavailable optional checks. Check committed OpenAPI/reference drift and preserve existing management
  API/client behavior and backend conformance.
