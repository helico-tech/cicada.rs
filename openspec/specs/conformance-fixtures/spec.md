# conformance-fixtures Specification

## Purpose
The language-neutral test data each release publishes, so the HTTP worker ports in other languages can check that
they match Rust exactly on canonical JSON, contract hashes, lease bodies and HTTP status handling.

## Requirements

### Requirement: Releases publish versioned portable fixtures

From this change onward, each new release SHALL include `cicada-fixtures-vX.Y.Z.tar.gz` beside the consumer skill.
It SHALL contain the four format-1 fixture files and tag provenance under `cicada-fixtures/`, with stable unique case
names. JSON inputs and bodies SHALL be strings that preserve numeric lexemes when the outer document is parsed.

#### Scenario: Exact tag contents

- **WHEN** the fixture archive is packaged with dirty or unrelated files in the caller's checkout
- **THEN** only the four regular fixture blobs at the tag commit and its derived SOURCE are included
- **AND** the only members are the root directory and `canonical.json`, `contracts.json`, `leases.json`, `status.json`
  and `SOURCE.json` beneath it; SOURCE names repository, tag, full commit and workspace version
- **AND** a missing/extra file, nonregular mode, unsupported format or tag/workspace mismatch refuses packaging
- **AND** packaging the same tag again produces the same bytes with fixed archive metadata and verified read-back

#### Scenario: Numeric input survives loading the fixtures

- **WHEN** a port loads a case with `1.0`, `-0` or an i64 extreme in its JSON input
- **THEN** the outer fixture exposes that input as text, without rounding or converting it to the port's default number

### Requirement: Fixed fixtures are checked against the implementation

The suite SHALL check canonical text and hashes, contract hashes, lease codecs and status mappings against reviewed
fixed expectations. It SHALL check file membership, format, unique case names and normalization references. It
SHALL NOT provide an update mode that silently replaces expected output after an implementation change.

#### Scenario: Canonical text and contract hashes

- **WHEN** the fixture tests run
- **THEN** every valid canonical case produces its exact text and prefixed hash and is idempotent on reparse
- **AND** every contract case produces its expected hash, which also hashes its expected stripped canonical text
- **AND** invalid canonical inputs are refused by the baseline parser

#### Scenario: Complete lease examples and normalization

- **WHEN** valid lease request/response samples are decoded and re-encoded by the Rust wire types
- **THEN** they reproduce the fixed Rust output, with explicit null body only for release's 204 response
- **AND** accepted inputs with response extension fields or omitted defaulted fields reproduce their named output case
- **AND** exhaustive enum classification plus required-case assertions covers every response, report, state, outcome
  and control variant
- **AND** samples cover i64 extremes, nullable fields, counters, effects, policies and delivery/duration forms
- **AND** this exact Rust output does not impose JSON member order or numeric spelling on other HTTP implementations

#### Scenario: Refused wire data

- **WHEN** invalid samples contain unknown request fields, missing required fields, unknown variants, bad numeric
  types/ranges, positional arrays, policies or durations
- **THEN** each fails at its declared decode or core-conversion stage
- **AND** successful decoding alone is not claimed to validate runtime lease admission or a report's effects

#### Scenario: Status classification

- **WHEN** the private client status classifier is checked against the data for statuses 100 through 599
- **THEN** invalid is 400/413/422, unavailable is 408/429/502/503/504, and the unexpected-status default is fatal
- **AND** a misleading status in the problem body does not change the class
- **AND** operation success codes are separately checked against the client and OpenAPI, with malformed success
  bodies failing fatally
- **AND** scripted client tests check the data's registration exceptions and vanished-registration follow-up

#### Scenario: Proof of drift detection

- **WHEN** a canonical expected digit or a saved expected hash is temporarily changed during implementation verification
- **THEN** the appropriate fixture check fails, and the temporary mutation is removed before committing
