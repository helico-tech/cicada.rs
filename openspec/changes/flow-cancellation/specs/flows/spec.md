## MODIFIED Requirements

### Requirement: Completed and stalled runs use ordinary controls

Completion SHALL set status done and armed null; done and cancelled runs SHALL ignore Continue and remain until deletion.
Cancelling a pending continuation message, purging it or exhausting its retries SHALL leave saved progress available;
another public Continue SHALL recover a still-running run, resetting attempts by arming a fresh message. Ordinary watches, pause, resume, delete and claim-limit
semantics SHALL apply. The runner SHALL install no watch of its own.

#### Scenario: Completed run

- **WHEN** a completed run receives Continue again
- **THEN** no closure or continuation runs and its saved results remain readable

#### Scenario: Stalled continuation

- **WHEN** a running run's armed message is cancelled, purged or fails terminally, then a public Continue is sent
- **THEN** a fresh armed message resumes from saved progress using the flow's policy

#### Scenario: Out of retries

- **WHEN** an armed step exhausts its retries
- **THEN** its message ends failed, core watchers are notified, and earlier saved entries remain

#### Scenario: Cancelled flow differs from a cancelled message

- **WHEN** FlowMsg::Cancel has committed cancelled status and the host sends Continue
- **THEN** normal work does not resume and the retained journal remains readable
- **AND** a new job requires a new run key
