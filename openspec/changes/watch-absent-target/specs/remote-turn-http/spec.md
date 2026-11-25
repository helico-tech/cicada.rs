## ADDED Requirements

### Requirement: A remote turn report carries a watch of an actor that must exist

A watch change in a remote turn report SHALL carry `"existing": true` for `watch_existing`, and SHALL leave the field
out otherwise; a body without it SHALL read as a plain watch change. A body with `existing: true` on a type target or
with `set: false` SHALL be refused as an invalid report.

#### Scenario: A worker over HTTP

- **WHEN** a remote worker runs a turn that calls `watch_existing` for an absent actor
- **THEN** the report holds `"existing": true`, and the store puts the same `Stopped` as a local commit

#### Scenario: An invalid watch change

- **WHEN** a remote turn report holds `{"target": "billing", "set": true, "existing": true}`
- **THEN** the report is refused as invalid and nothing is committed
