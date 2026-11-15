# 0047. Canonical JSON and conformance fixtures

Date: 2026-11-15

## Status

Accepted for `conformance-fixtures`. Adds the tested canonical-text convention and a release asset to ADRs 0017,
0041 and 0046; it changes no existing hash, runtime wire format or guarantee.

## Context

External worker implementations need the exact contract-hash inputs and representative lease exchanges. Rust tests
and schemas contain the facts, but are not portable test data. In particular, JavaScript's default number printing
and UTF-16 key order are not Cicada's canonical JSON. The scalar writer currently delegates to serde_json, so a
consumer's resolved dependencies and merged features can matter independently of this repository's lockfile.

A proposed precision-search float formatter is not compatible: binary rounding intervals are asymmetric at powers
of two, and the nearest decimal at a given precision can miss a shorter decimal that round-trips. Boundary testing
found different digits. Publishing fixtures must not become an unproven replacement of the hashing implementation.

## Decision

- Keep `canonical_json` and the parser unchanged. Objects sort keys by UTF-8 bytes, without Unicode normalization;
  arrays keep order and no insignificant whitespace is written. Strings escape quote and backslash, use the short
  escapes for backspace/form feed/newline/carriage return/tab, and lowercase `\u00xx` for other controls below U+0020.
  Other characters, including slash, U+007F and U+2028, remain raw UTF-8.
- Represented i64/u64 numbers print as decimal integers. Represented finite binary64 numbers use shortest
  round-tripping digits, choosing nearest digits and ties to even; decimal exponents -5 through 15 use fixed notation
  with `.0` for whole floats. Others use a lowercase `e`, an explicit exponent sign and no leading exponent zeroes.
  Negative float zero remains `-0.0`. This is not a byte-preserving representation of the input JSON.
- Parsing determines the represented number: `0` prints `0`, `-0` prints `-0.0`, `1e2` prints `100.0`; integers beyond
  i64/u64 can become binary64, underflow can become zero, and nonfinite overflow is refused. The recorded baseline
  is serde_json 1.0.151 with preserve_order and float_roundtrip, zmij 1.0.23, and no arbitrary_precision feature.
  Other dependency/feature combinations are not certified by these fixtures; their current behavior is unchanged.
- Hashes remain `sha256:` followed by lowercase hex over canonical UTF-8 bytes. Contract hashes first remove only
  schema description keywords, as ADR 0017 specifies; names and data called `description` remain.
- Commit fixed, reviewed expectations in `fixtures/{canonical,contracts,leases,status}.json`. Each declares
  `format: 1`. JSON inputs, contract schemas and HTTP bodies are strings in these files, so loading the outer file
  does not lose numeric spelling or large integers. Unique case names are stable references. No update switch
  regenerates expectations from a changed implementation.
- Canonical and contract cases assert exact text/hash. Lease cases include the four operations, all tagged
  alternatives, normalized tolerant/defaulted inputs and explicit decode/conversion failures. Their exact output
  is a Rust serializer regression check, not an HTTP requirement on member order or numeric spelling. Status data
  describes the real status classification, success codes and registration exceptions of ADR 0040.
- Release `cicada-fixtures-vX.Y.Z.tar.gz` beside the skill archive. It contains `cicada-fixtures/` with the four files
  and `SOURCE.json` (repository, tag, commit, version). A deterministic packager reads regular blobs at the tag,
  checks exact membership and workspace version, and verifies the archive on read-back. The existing skill command
  and archive remain compatible. Releases before this change gain no backfilled asset.
- The local release driver verifies both packages before pushing the tag, saves their filename-to-digest map, and
  refuses any changed map on retry. It refuses a legacy singular digest state instead of silently converting it;
  interrupted old releases use their pinned old runner. Gate evidence records keep their existing schema.
- A matching draft may have any subset of the two assets. Upload only the missing ones, re-read even after an
  uncertain upload, and verify the full set before publishing. Duplicate, extra or conflicting assets stop the
  operation without overwrite. Published releases, immutable tags and all other ADR 0046 protections stay intact.

## Consequences

The release gives ports independently consumable compatibility examples and detects covered drift before publishing.
Finite vectors are not a proof for every number or every consumer's dependency graph. A dependency update must pass
the fixed expectations; an intentional canonical-format change requires a new ADR and an explicit hash-compatibility
plan. Exact dependency pins and a custom formatter are not introduced here. A later formatter change needs its own
proposal and proof, including binary-boundary and merged-feature behavior.

Fixture format 1 is a versioned data interface: adding cases is compatible, changing its field meanings needs a new
format. Tests check case references, enum coverage, codecs and status mappings. Existing contract snapshots pass
without regeneration. The builder hands consumer usage, tested dependency limits and release instructions to the
docs session; no new guide or public API is needed to ship the data.
