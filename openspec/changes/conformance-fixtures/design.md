# Design

## Scope and canonical text

1. Publish checked compatibility data, with five build tasks plus finishing. Keep `canonical_json`, parsing,
   contracts and HTTP behavior unchanged. The proposed precision-search float writer is rejected: at binary
   boundaries its nearest decimal can miss a shorter round-tripping decimal; the adviser found 46 mismatches among
   6,138 boundary values. A new formatter needs separate compatibility design, not this fixture release.
2. ADR 0047 records today's format: UTF-8 byte key order, no normalization or insignificant whitespace, array order
   retained; serde_json escaping; decimal i64/u64; shortest round-tripping finite floats, nearest digits with ties
   to even, fixed notation for decimal exponents -5 through 15 with `.0` for whole floats, otherwise signed exponent.
   Keep negative float zero. Parse classification matters: `0` differs from `0.0`, and `-0` parses as `-0.0`.
   The tested baseline is serde_json 1.0.151 with preserve_order/float_roundtrip and zmij 1.0.23, without
   arbitrary_precision. Record this in the data. Other consumer resolutions/features are not certified by this
   suite; no new runtime refusal is added. Fixed vectors detect covered drift, not every possible upstream change.
   No exact dependency pins: they constrain consumers and still cannot freeze parser/toolchain behavior.

## Fixture contract

3. Use four files in root `fixtures/`: `canonical.json`, `contracts.json`, `leases.json`, `status.json`.
   Each is an object with `format: 1`; names within each case list are unique and stable. No release version in the
   files: `SOURCE.json` in the archive supplies it. Inputs, schemas and wire bodies are JSON text in strings, never
   pre-parsed numbers in the outer data. Expected values are reviewed literals, with no update variable/generator
   that accepts new expectations. A builder may obtain initial examples from Rust, then fixes them as test vectors.
4. `canonical.json` holds `baseline` (the dependency versions/features above), `valid` cases
   `{name,input,canonical,hash}` and `invalid` cases `{name,input}`. `hash` is `canonical_sha256`, including `sha256:`.
   Cover escapes, raw U+007F/U+2028, UTF-8 versus UTF-16 order, composed/decomposed keys, arrays, literals, signed zero,
   i64/u64 edges, out-of-integer-range parsing, subnormal/normal/max floats, underflow, overflow, notation thresholds,
   ties and the discovered power-of-two counterexample. Reject malformed JSON/unpaired surrogates; omit duplicate keys.
   `contracts.json` holds `cases: [{name,input,canonical,hash}]`: full contract JSON text, stripped canonical text and
   hash. Cover schema descriptions at every traversed keyword, and retained data/name keys called `description`.
5. `leases.json` holds `valid`, `accepted` and `invalid` lists. Valid cases are
   `{name,operation,direction,status,body}`: operation lease/renew/release/complete, request status null, response
   status 200 (release 204); only the empty 204 response has body null. Bodies are Rust's compact serialized output.
   Accepted cases `{name,operation,direction,input,expected}` name a valid case's normalized output, covering unknown
   response fields and omitted/defaulted request fields. Invalid cases `{name,operation,direction,input,stage}` name
   decode or core-conversion rejection, not arbitrary runtime validation. No claim that all decodable requests work.
   Cover all result/report/state/outcome/control variants, all effects, nullable fields, i64 extremes, u32 boundaries,
   delivery modes, nanosecond durations, retry/retention policies, unknown/missing/mistyped fields and invalid policies.
   Ports need not emit Rust's field order/number spelling: these bytes are input examples, not a new HTTP constraint.
6. `status.json` holds `success` by operation, `unexpected` (invalid/unavailable arrays and fatal default), and
   `registration` exception rows naming status, problem type, follow-up and result. Status alone chooses the general
   error; malformed expected bodies are fatal. Include the 422 invalid-contract, 409 contract-conflict plus GET, and
   vanished-registration 404 no-such-type cases. Transport/timeouts remain existing prose, not invented HTTP statuses.

## Checks

7. `crates/cicada/tests/fixtures.rs` uses public APIs: byte-equal canonical text/hash, canonical idempotence, and
   contract hash plus expected stripped text hashing to it. Require that expected stripped text is itself canonical
   and SHA-256 its exact UTF-8 bytes, not only its reparsed value. Typed wire decode/re-encode must equal the chosen valid
   output exactly, including for normalized accepted cases. Check conversions for the cases marked conversion;
   do not assert serde rejects a semantic error only a later conversion checks. No `Value::eq` as numeric proof.
   Explicit exhaustive matches classify the response/report/state/outcome/control enums, with a required case for
   each arm; no recursive schema-enum crawler. Check fixture file names and format, unique names and references.
8. A unit test in `cicada-client/src/turns.rs` compares `unexpected()` against status data for 100–599 without sockets,
   including misleading problem status fields. Existing scripted-server `tests/turn_source.rs` consumes registration
   rows and success statuses to prove the exceptions/actual operations. Axum's existing OpenAPI test reads success
   rows and asserts the documented route status, not a cross-crate import into core. Keep runtime tests virtual-time
   as their existing harness permits. Fixed examples do not constitute a general conformance runner or proof of G/S.

## Packaging and resumable release

9. `scripts/package-fixtures.py vX.Y.Z OUTPUT_DIR` writes `cicada-fixtures-vX.Y.Z.tar.gz` from tag objects only.
   Exact members: root directory `cicada-fixtures/`, the four JSON files, and `SOURCE.json` with repository/tag/commit/
   version as in the skill. Refuse missing/extra files, nonregular Git modes, directories/symlinks in `fixtures/`,
   bad fixture formats and a tag/workspace version mismatch. Reuse deterministic tar/gzip/read-back helpers from
   `package-skill.py` through a small shared Python module; keep the skill command, contents and bytes unchanged.
   Load the sibling helper explicitly (the gate uses `python3 -I -B`); exercise both isolated entry points and the
   existing importlib-based tests, and prove that the same tag still yields identical skill archive bytes.
10. Gate packages and verifies both archives before tag push: exact member set/types, provenance and tag blob bytes.
    Persist the complete `assets: {filename: sha256}` map atomically only after both verify; retries compare, never
    replace a saved digest or set. Refuse legacy `asset_sha256` state with an actionable pinned-runner diagnostic;
    do not bump evidence-record schema or reinterpret old state. A pre-package state with neither field may proceed.
    An old release resumes with its own pinned runner. Already published releases remain untouched, without backfill.
11. Draft verification accepts a subset of the two expected names, once each, with matching digests; foreign names,
    duplicates, different content/title/notes/tag stop without clobber. Create with both; upload only missing assets.
    Re-read after each upload (also uncertain failure), and verify the complete set before publishing. Preserve raw
    tag identity, exact-commit evidence, locks and remote-race checks. Tests fake GitHub and use real local Git objects;
    a live GitHub upload is not a prerequisite claimed by the design.

## Documentation and verification boundaries

12. ADR 0047 and its index are producer-owned. Handoff architecture's hash rule, HTTP guidance for ports, releasing's
    two archives and AGENTS map, plus normal version pins. No new guide, example, public API, schema or route is added;
    no known doctest/link/guarantee/skill-route blocker. Root JSON data needs no skill route. Use the next patch version.
    Fixed contract snapshots must pass without update variables. No code changes outside tests, packaging and necessary
    comments/version metadata. Gate evidence comes from the builder's final committed tree, never the scratch experiments.
