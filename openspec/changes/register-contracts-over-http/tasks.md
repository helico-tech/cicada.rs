## Implementation

- [x] 1. Add ActorSystem::register_contract and RegisterContractResult, with validation of schema roots, message
  names, NUL and the separate 120-level schema bound before recomputing the hash. Use Backend::register without a
  preliminary read, return its stored definition and leave local initialization unchanged. Make TypeResolver reuse
  a hash entry only when its complete source JSON matches. Add public-API tests in crates/cicada/tests/registration.rs
  for unstarted/empty systems, no hosting or execution, generated and supplied hashes, equal/conflicting/concurrent
  registrations, original descriptions, and peer delete/re-register/read through populated caches. Check 120-level
  state and message schemas through SQL reopen/read/equal retry and reject 121 levels without writes; cover NUL in
  keys and values, boolean roots, empty messages and invalid roots/names. Use virtual time for runtime behavior;
  cover memory, SQLite and PostgreSQL when CICADA_PG_URL is set. Run the registration tests with sqlite,postgres,testing
  enabled and the existing registry conformance tests; do not change backend registration or the schema.
- [x] 2. Add the strict RegisterTypeRequest wire type, PUT /types/{name} handler on the full router, and the
  invalid-contract/contract-conflict problem constants. Return the stored TypeView with 200 for both new and equal
  requests; retain existing parsing, size, closed-store and read-router rules. Extend core api tests and
  cicada-axum/tests/api.rs for required fields, forbidden identity/hash fields, malformed paths/bodies, 422 validation,
  409 conflict, 413 size, 503 close, read-only 405, retry, and create/send schema gating without a handler. Check
  descriptions after peer replacement and deep response JSON. Add operationId registerType and request/response
  schemas to OpenAPI; extend tests/openapi.rs for full/read differences and regenerate docs/openapi.json with
  CICADA_UPDATE_OPENAPI=true. Run the affected core API, axum API and OpenAPI tests.
- [x] 3. Add ActorClient::register_type(&Contract) -> ClientResult<TypeView>, using the contract's type name in
  the path and only state/messages in the body. Extend cicada-client/tests/client.rs for a generated Rust contract,
  equal registration returning original descriptions, ordinary problem errors on conflict/invalid contract and
  the 120-level schema boundary through both get_type and types. Ensure no supplied hash crosses the wire and no
  problem detail is parsed. Run the targeted client tests and the documented-routes coverage check.
- [ ] 4. Update architecture and the core/HTTP guides for registration without hosting, first-registration-wins,
  schema versus value limits, validator fallback, deferred execution and trusted write access. Add a tested example
  using contract_of rather than a handwritten approximation; it must keep the generated hash equal when a local
  host later initializes. Update the axum route table, ADR 0036/index as needed, CHANGELOG.md and workspace version.
  Run `cargo test -p cicada-examples --doc`, `cargo test -p cicada-examples --test docs` and affected crate doctests.
  No actor contract, CLI or database migration change belongs here.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate register-contracts-over-http --strict`; include PostgreSQL with CICADA_PG_URL when available
  and record any unavailable optional checks. Commit generated OpenAPI and any version-bearing reference updates
  required by existing drift checks. Verify accepted schema-boundary round trips on SQL, not only memory.
