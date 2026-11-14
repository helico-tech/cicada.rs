## MODIFIED Requirements

### Requirement: Saved wait outcomes have consistent reply selections

An open wait SHALL have empty received. A closed wait's received SHALL be a duplicate-free subset in request order,
with stored reply values. Received/all SHALL include every request and received/first exactly one. TimedOut SHALL
require a deadline; timedOut/all SHALL include a strict subset and timedOut/first none. These are structural rules;
this release SHALL execute single and joined all/first waits with or without a deadline in the active running or cancelling phase.

#### Scenario: Invalid outcome

- **WHEN** an open wait selects replies, a selected value is absent, selections are repeated/out of order, all/first
  cardinality is wrong, or timedOut has no deadline
- **THEN** typed decoding refuses the state

#### Scenario: Partial timeout and interrupted main wait

- **WHEN** an all wait with a deadline times out with a strict subset of stored replies in request order
- **THEN** its closed outcome is structurally valid
- **AND** a cleanup wait can record that outcome while its interrupted main wait remains open under cancelling/cancelled

### Requirement: Completed and stalled runs use ordinary controls

Main completion SHALL set status done and armed null; done and cancelled runs SHALL ignore Continue and remain until deletion.
Cancelling a pending continuation message, purging it or exhausting its retries SHALL leave saved progress available;
another public Continue SHALL recover a running or cancelling run in its active phase, resetting attempts by arming a fresh message. Ordinary watches, pause, resume, delete and claim-limit
semantics SHALL apply. The runner SHALL install no watch of its own.

#### Scenario: Completed run

- **WHEN** a completed run receives Continue again
- **THEN** no closure or continuation runs and its saved results remain readable

#### Scenario: Stalled continuation

- **WHEN** a running or cancelling run's armed message is cancelled, purged or fails terminally, then a public Continue is sent
- **THEN** a fresh armed message resumes the active phase from its saved progress using the flow's policy

#### Scenario: Out of retries

- **WHEN** an armed step exhausts its retries
- **THEN** its message ends failed, core watchers are notified, and earlier saved entries remain

#### Scenario: Cancelled flow differs from a cancelled message

- **WHEN** successful cleanup has committed cancelled status and the host sends Continue
- **THEN** normal work does not resume and the retained journal remains readable
- **AND** a new job requires a new run key

