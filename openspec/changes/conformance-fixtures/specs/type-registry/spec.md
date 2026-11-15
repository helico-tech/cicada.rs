## ADDED Requirements

### Requirement: Canonical contract text has a tested written format

Canonical JSON SHALL follow ADR 0047's recorded format and tested baseline. Contract hashing SHALL retain its
description stripping and `sha256:`-prefixed SHA-256 over canonical UTF-8 text. This change SHALL retain the current
parser and formatter. Fixed vectors SHALL detect covered drift, without claiming compatibility for every value or
other consumer dependency/feature combinations.

#### Scenario: Integer and float representations

- **WHEN** baseline JSON inputs `0`, `0.0`, `-0`, `1e2` and `18446744073709551616` are parsed and canonicalized
- **THEN** their texts are respectively `0`, `0.0`, `-0.0`, `100.0` and `1.8446744073709552e+19`
- **AND** vectors include signed/unsigned integer boundaries, subnormals, underflow and refused nonfinite overflow

#### Scenario: Float digits and layout

- **WHEN** a finite float is canonicalized under the recorded baseline
- **THEN** shortest round-tripping digits use fixed notation for decimal exponents -5 through 15, otherwise a
  lowercase exponent with an explicit sign
- **AND** whole floats in fixed notation have `.0`, and negative zero keeps its sign
- **AND** fixed cases cover both notation thresholds, decimal ties and powers of two with asymmetric rounding intervals

#### Scenario: Keys and string escapes

- **WHEN** keys include U+E000 and U+1F600, or composed and decomposed spellings of a character
- **THEN** UTF-8 byte order applies and the different spellings remain distinct
- **AND** arrays retain their order and canonical text has no insignificant whitespace
- **WHEN** strings contain controls, quotes, backslashes, slash, U+007F or U+2028
- **THEN** the escaped text matches ADR 0047 and the fixed vectors exactly

#### Scenario: Hash inputs and descriptions

- **WHEN** contract vectors differ only in schema description keywords
- **THEN** their fixed hashes agree
- **AND** fixtures also show that a property or message named `description`, or a description key inside literal
  schema data, remains part of the hash input

#### Scenario: Dependency drift

- **WHEN** resolved dependencies change a canonical output or hash covered by a fixed vector
- **THEN** the fixture check fails without automatically rewriting the expectation
- **AND** the fixture baseline names the tested versions/features and does not certify a consumer's other resolution
