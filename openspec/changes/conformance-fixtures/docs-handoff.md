# Docs handoff for conformance-fixtures

Version: 0.18.12. Refresh the dependency pins of `docs/how-to/install.md` once it is published (and its skill
download example, which still names v0.18.10). No public Rust API, HTTP route, wire format, contract hash, store
schema, guarantee or CLI changes.

## What changed

- `fixtures/` at the repository root holds four fixed, reviewed data files, each a JSON object with `format: 1`, with
  every JSON input, contract and HTTP body as a string, so a port's JSON loader keeps numeric spelling and large
  integers:
  - `canonical.json`: `baseline` (serde_json 1.0.151 with `float_roundtrip` and `preserve_order`, without
    `arbitrary_precision`; zmij 1.0.23), `valid` cases `{name, input, canonical, hash}` and `invalid` cases
    `{name, input}` the parser refuses.
  - `contracts.json`: `cases` `{name, input, canonical, hash}`: a contract's JSON, the canonical text of what is
    hashed (schema descriptions removed) and its `sha256:` hash.
  - `leases.json`: `valid` `{name, operation, direction, status, body}` (Rust's exact output for the four lease
    operations; `status` null for requests, 200, or 204 with body null for release), `accepted`
    `{name, operation, direction, input, expected}` (inputs that normalize to the named valid case), and `invalid`
    `{name, operation, direction, input, stage}` with stage `decode` or `conversion`.
  - `status.json`: `success` per operation (method, path, operationId, status), `malformedSuccess`, `unexpected`
    (`invalid` 400/413/422, `unavailable` 408/429/502/503/504, `default` fatal) and the `registration` exception rows.
- Tests: `crates/cicada/tests/fixtures.rs` (canonical text, hashes, contract hashes, lease codecs with exhaustive
  variant coverage, file membership and references); `every_unexpected_status_is_classified_as_the_fixture_says_whatever_the_body_claims`
  in `crates/cicada-client/src/turns.rs`; `each_operation_succeeds_only_at_its_fixture_status_and_a_malformed_success_is_fatal`
  and `registration_follows_the_exception_rows_of_the_fixture_and_nothing_else` in
  `crates/cicada-client/tests/turn_source.rs`; `the_success_statuses_of_the_fixtures_are_the_documented_ones` in
  `crates/cicada-axum/tests/openapi.rs`. No variable rewrites the fixtures.
- Release: `scripts/package-fixtures.py vX.Y.Z DIR` writes `cicada-fixtures-vX.Y.Z.tar.gz` (`cicada-fixtures/` with
  the four files and `SOURCE.json`: repository, tag, commit, version) from the tag's Git objects. It shares
  `scripts/release_archive.py` with `scripts/package-skill.py`, whose command, contents and bytes are unchanged.
  `scripts/gate.py release` now packages and verifies both archives before pushing the tag, saves their
  name-to-digest map (`assets`) in the release state, refuses a retry whose archives hash differently, resumes a
  matching draft that holds any subset of the two assets by uploading only the missing ones, and refuses a state with
  the old single `asset_sha256` field. Tests: `scripts/test_package_fixtures.py`, `scripts/test_gate_release.py`,
  `examples/tests/skill/test_package_skill.py`.
- ADR 0047 and its index row were committed with the design and match the built data; no edit was needed.

## 1. The canonical text behind contract hashes

**Where.** `docs/architecture.md`, `Domain language`, the `Contract` row, or a sentence under `HTTP contract` where
the type registry is described.
**Text.** The hash is `sha256:` and the lowercase hex SHA-256 of the contract's canonical JSON, after removing the
keyword `description` from the state and message schemas and every subschema under a keyword that holds schemas
(ADR 0017). Canonical JSON (ADR 0047): object keys sorted by their UTF-8 bytes, no Unicode normalization, no
insignificant whitespace, arrays in order; strings escape `"` and `\`, use `\b \f \n \r \t` and lowercase `\u00xx` for
the other controls below U+0020, and keep everything else raw (`/`, U+007F and U+2028 included); i64/u64 integers in
decimal; floats in their shortest round-tripping digits, fixed notation for decimal exponents -5 to 15 with `.0` on
whole numbers, otherwise `e`, a sign and the exponent (`1e+16`, `1e-6`), `-0.0` kept. Parsing decides the number:
`0` stays `0`, `-0` and `1e2` become floats (`-0.0`, `100.0`), integers beyond u64 become floats. The release's
fixture archive holds the vectors.
**Caveats.** The format is the output of serde_json 1.0.151 with `preserve_order` and `float_roundtrip` and zmij
1.0.23; a consumer whose build resolves other versions or merges `arbitrary_precision` is not covered by the
fixtures. A change of the format would be a new ADR with a hash-compatibility plan.

## 2. Testing another implementation against the fixtures

**Where.** `docs/guide/http.md`, `Run a worker over HTTP`, after the status table (or a short subsection
`Check another implementation`).
**Text.** Each release carries `cicada-fixtures-vX.Y.Z.tar.gz`, beside the skill archive, for implementations of the
worker protocol in other languages: `canonical.json` and `contracts.json` (canonical text and contract hashes),
`leases.json` (request and response bodies of `/leases`, `/leases/renew`, `/leases/release` and `/leases/complete`:
valid, accepted-and-normalized, and refused at decode or conversion) and `status.json` (the success status of each
operation, the classification in the table above, and the registration exceptions: `422 invalid-contract`,
`409 contract-conflict` with one `GET /types/{name}`, and its `404 no-such-type` follow-up, which is unavailable).
Every input is a JSON string: parse it with a parser that keeps integers and floats apart. Download with
`gh release download vX.Y.Z --repo helico-tech/cicada.rs --pattern 'cicada-fixtures-*.tar.gz'`.
**Caveats.** Rust's member order and number spelling in `leases.json` are examples, not a requirement; compare by
meaning, except for canonical text and hashes, which must match byte for byte. A decoded body is not a promise that a
server admits the lease or applies a report's effects. The fixtures exist from 0.18.12; earlier releases have none.

## 3. Two release archives

**Where.** `docs/how-to/releasing.md`: step 3, step 4, `Resume a release` and its last paragraph.
**Text.** Step 3: "packages and verifies the skill and the fixtures" and "verifies its notes and both assets". Step 4:
"Confirm the published Release has `cicada-skill-vX.Y.Z.tar.gz` and `cicada-fixtures-vX.Y.Z.tar.gz`." Resume: the
state saves both archives' digests (`assets`); a retry whose archives hash differently is refused; a matching draft
may hold either asset or both, and only the missing one is uploaded; a state with the single `asset_sha256` of an
earlier driver is refused: resume that release with the `scripts/gate.py` of its pinned commit. Last paragraph:
`python3 scripts/package-fixtures.py vX.Y.Z dist` reproduces the fixture archive the same way.
**Caveats.** Published releases are never changed: no fixture archive is added to releases before 0.18.12.

## 4. AGENTS.md

**Where.** `AGENTS.md`, `Build and test` and `Map`.
**Text.** Build and test: `cargo test -p cicada --test fixtures   # the fixtures in fixtures/ against the code; no
update variable (ADR 0047)`. Map: a row `fixtures/` — "Portable test data for other implementations (ADR 0047):
canonical JSON, contract hashes, lease bodies and status classification; fixed, reviewed, checked by
`crates/cicada/tests/fixtures.rs`". The `scripts/` row: add `package-fixtures.py`, the fixture archive, and
`release_archive.py`, the helpers both packagers share.
**Caveats.** None.

## Blocking

None. Every doctest, link, guarantee row and skill route passes without prose edits.

## Status

Proposed text only; no docs-owned file is edited and no owner fix is integrated, and none of the prose above is
written yet. Checks on 8591bc0: `python3 scripts/gate.py run` and `run --e2e` passed (full and e2e evidence for tree
7792f876ad41: script tests, fmt, clippy with `-D warnings`, `cargo test --workspace` with PostgreSQL, feature builds,
the app alone, dependency trees and the Docker E2E suite). Run directly as well: `cargo test --workspace` with
PostgreSQL (2957 passed, 0 failed); `cargo test -p cicada-examples --test skill --test docs` and `--doc` (pass);
`openspec validate conformance-fixtures --strict` (valid). Contract snapshots (`cicada-test`, the actors'
`wire_formats`) pass without update variables. Drift detection: changing `1e+16` to `1e+17` in `canonical.json` and
one digit of the `minimal` hash in `contracts.json` failed `exponent-at-16: canonical text` and `"minimal": contract
hash`; changing `"factor":2.0` to `2` in `leases.json` failed the valid and accepted lease tests; all were restored
before committing. The packaging tests use temporary repositories and a fake `gh`; no live GitHub release was made.
