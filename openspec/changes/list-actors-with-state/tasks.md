## Implementation

- [x] 1. Add private Backend::list_records and its store-handle forwarding, with one explicitly projected actor
  SELECT per SQL read attempt and one locked memory read. Reuse each backend's existing prefix/cursor rules, row
  decoder and page_of without making address-only list fetch state. Add and register conformance cases in
  `s1_to_s10.rs` for rows equal to get, addresses/cursors equal to list, empty/last pages, literal percent/underscore
  prefixes and overlapping type names, before/after-range cursors, positive core limits including 1001, and coherent
  committed state/version and multi-row effects. Run targeted conformance tests on memory, SQLite and PostgreSQL
  with CICADA_PG_URL; document unavailable fixtures. Add new tests proving S2/S3 to docs/guarantees.md.
- [x] 2. Add ActorSystem::list_actors_with_state returning Page<ActorSnapshot>, sharing get_actor's private conversion.
  Add virtual-time TestActors cases in `tests/runtime/operations.rs` for versions after turns, uncommitted state,
  parent/paused metadata, unhosted types, direct limits 0/1001 and changes between pages. Extend the legacy-depth
  fixture in `tests/runtime/value_depth.rs` to list older states without applying write validation. Run these runtime
  tests and `cargo check -p cicada` without features.
- [x] 3. Add GET /actors/states on both routers with operationId listActorStates, existing ListQuery and fixed
  PageView<ActorView>, plus ActorClient::list_actors_with_state. Extend axum API and client tests for full fields,
  pagination, empty results, limits/defaults/duplicates, literal prefixes, type states route coexistence, store error
  mapping, maximum-depth state round-trip and unchanged GET /actors. Regenerate docs/openapi.json using
  CICADA_UPDATE_OPENAPI=true and verify both routers' documents. Run the affected axum/client tests.
- [ ] 4. Document the method/route in crate docs and core/HTTP guides with compiling listing examples. Explain
  per-page read consistency, keyset concurrency, version lifetime, core versus HTTP limits, large-page sizing,
  legacy-depth caveats and new-server requirement. Add the route to the HTTP depth paragraph (state + 3 = 67), and
  describe this listing in architecture with ADR 0033. Update changelog/version using the release process; leave CLI
  and actor contracts unchanged. Run affected doctests, `cargo test -p cicada-examples --doc` and
  `cargo test -p cicada-examples --test docs`, including the updated guarantee-to-test links.

## Finishing checks

- [ ] 5. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` and
  `openspec validate list-actors-with-state --strict`; include PostgreSQL where configured and record any unavailable
  optional checks. Regenerate version-bearing references if their existing drift checks require it.
