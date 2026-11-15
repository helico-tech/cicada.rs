## ADDED Requirements

### Requirement: The lease request documents the limits the core checks

The rustdoc and generated OpenAPI descriptions of `LeaseTurnsRequest` SHALL state the type-count, batch and
owner-length limits enforced by `ActorSystem::lease_turns`. A test SHALL compare the generated descriptions against
the runtime constants, and the committed OpenAPI SHALL match the generated document. This requirement SHALL NOT
change request validation or schema constraints.

#### Scenario: Documented limits match the checks

- **WHEN** OpenAPI is generated
- **THEN** the `types` description states 1 to `MAX_LEASE_TYPES` types, the `limit` description states 1 to
  `MAX_LEASE_BATCH`, and the `owner` description states at most `MAX_ID_LENGTH` characters
- **AND** the type-count and batch descriptions name their corresponding constants
- **AND** `docs/openapi.json` carries the same descriptions

#### Scenario: A limit changes without its documentation

- **WHEN** one of those constants changes without correcting the corresponding rustdoc description
- **THEN** the generated-description test fails with the property and expected phrase

#### Scenario: Corrected rustdoc has not been regenerated

- **WHEN** a lease description changes but the committed OpenAPI still has the old text
- **THEN** the existing document freshness test fails until the generated document is updated
