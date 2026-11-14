## ADDED Requirements

### Requirement: JSON text parsing preserves finite floating-point round trips

Cicada crates SHALL enable serde_json's `float_roundtrip` feature through their shared dependency so that serializing
and parsing a finite f64 adds no rounding error. Tests SHALL verify the feature without relying on unrelated workspace
dev dependencies. This SHALL NOT promise original JSON spelling or override a database's own numeric normalization.

#### Scenario: Numeric regression across text storage

- **WHEN** states and message payloads containing `1.0715660391465826e-75`, its negative, subnormal and finite maximum
  f64 values and boundary i64/u64 integers are written and read through each backend and copied into failure notices
- **THEN** the parser introduces no numeric change, including through subsequent dead-letter text conversion
- **AND** memory and SQLite preserve negative-zero bits; PostgreSQL's JSONB signed-zero normalization is explicitly tested and documented

#### Scenario: A downstream crate

- **WHEN** a separate consumer depends on the released Cicada crates with their normal features
- **THEN** serde_json has float_roundtrip enabled for their JSON read paths without requiring a consumer feature opt-in
