# Design

## Scope

Correct the published descriptions of the existing lease limits. `MAX_LEASE_TYPES` and `MAX_LEASE_BATCH` are both
100; `LeaseTurnsRequest` incorrectly documents 64 types and 256 turns. No limit, validation, schema constraint,
wire shape, contract hash, store behavior or guarantee changes. This fits two build tasks plus finishing.

## Decisions

1. Keep the descriptions in `crates/cicada/src/api/leases.rs`, which schemars already copies into OpenAPI.
   The types sentence says `1 to 100 types` and names `MAX_LEASE_TYPES` in a plain code span. The limit sentence is
   `The most turns to lease, 1 to 100 (` followed by the code span `MAX_LEASE_BATCH` and `).`.
   Plain code spans work in both rustdoc and OpenAPI; Rust intra-doc links would expose unresolved Rust paths in
   the generated document. Do not introduce a separate computed schema description: rustdoc would still be manual.

2. Add `the_lease_request_documents_the_limits_the_core_checks` to
   `crates/cicada-axum/tests/openapi.rs`. Read the properties of `LeaseTurnsRequest` from `openapi_document()` and
   compare their descriptions with phrases formatted from `cicada::runtime::{MAX_LEASE_TYPES, MAX_LEASE_BATCH}`.
   Check the owner phrase against `cicada::runtime::limits::MAX_ID_LENGTH` too: it is the third numeric limit in
   the same request. Include the word or punctuation after each number, so an expected 100 cannot match 1000.
   Diagnostics identify the property and expected phrase. This checks the published text against the code, rather
   than asserting two copied numeric literals. Keep the existing runtime test that pins the actual limits at 100.

3. Write that test first and record its failure against the current descriptions. Correct the two rustdoc comments,
   regenerate with `CICADA_UPDATE_OPENAPI=true cargo test -p cicada-axum --test openapi`, then run without the variable.
   `the_committed_document_is_current` already proves that `docs/openapi.json` equals the generated document.
   No source-text parser, extra runtime boundary tests or temporary constant mutation is needed.

4. Keep schema validation keywords unchanged, including the generated `minimum: 0` for `usize`.
   Adding `maximum`, `maxProperties` or a stricter minimum would broaden this correction into a schema change.
   The existing core checks remain authoritative. No new dependency or public API is needed.

## Audit

- `owner` correctly says at most 200 characters and no NUL, matching `MAX_ID_LENGTH` and `check_lease_request`.
- Renew and release describe actor and epoch without incorrect numeric limits.
- Completion's failed report correctly allows retryability only for handler, node and timeout, and refuses
  claim-limit, matching `completion.rs`. Its request and response descriptions need no correction.
- The core guide's remote lease section already states 1–100 for both counts. The `turn-leases` spec and ADR 0039
  agree. The route summary contains no conflicting numbers. Leave these texts alone.
- The adjacent `conformance-fixtures` change owns language-neutral fixtures and packaging; this change creates none.

## Proof and limitations

The new synchronous test covers generated schema descriptions; the existing snapshot test covers the committed
OpenAPI. Together they catch a changed constant with stale rustdoc, and corrected rustdoc with a stale generated
file. A harmless rewording may require updating the expected phrase; that is an acceptable, focused maintenance cost.
The design audit read source and generated JSON; it did not compile or run tests. The builder supplies the red/green
evidence. Runtime behavior and backend conformance are unchanged.

## Documentation, ADR and release

The builder edits rustdoc, the generated OpenAPI, the test, CHANGELOG and normal version metadata. Write
`docs-handoff.md` with `Blocking: none` and no user prose change: the core guide already gives the correct limits.
No doctest block, link, guarantee row or skill route breaks. Do not edit README, AGENTS.md or handwritten docs.
No ADR is needed because this corrects descriptions without changing a guarantee or wire format.
Use the next patch version at build time, updating the consumer skill version through the normal release process;
do not reserve a version while other sessions can release. This is a normal build-loop release, not exempt docs
maintenance. Finishing uses the committed-tree local gate and strict OpenSpec validation.
