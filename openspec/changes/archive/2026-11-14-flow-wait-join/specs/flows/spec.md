## MODIFIED Requirements

### Requirement: Saved wait outcomes have consistent reply selections

An open wait SHALL have empty received. A closed wait's received SHALL be a duplicate-free subset in request order,
with stored reply values. Received/all SHALL include every request and received/first exactly one. TimedOut SHALL
require a deadline; timedOut/all SHALL include a strict subset and timedOut/first none. These are structural rules;
this release SHALL execute single and joined all/first waits with or without a deadline in the running phase.

#### Scenario: Invalid outcome

- **WHEN** an open wait selects replies, a selected value is absent, selections are repeated/out of order, all/first
  cardinality is wrong, or timedOut has no deadline
- **THEN** typed decoding refuses the state

#### Scenario: Partial timeout and interrupted main wait

- **WHEN** an all wait with a deadline times out with a strict subset of stored replies in request order
- **THEN** its closed outcome is structurally valid
- **AND** a cleanup wait can record that outcome while its interrupted main wait remains open under cancelling/cancelled

