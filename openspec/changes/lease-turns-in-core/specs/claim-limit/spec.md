## ADDED Requirements

### Requirement: External lease admission enforces the server claim limit

lease_turns SHALL filter messages at the serving system's max_lost_claims before handoff. For each, it SHALL attempt
one fenced failed commit without loading a handler or asking its retry policy. A confirmed commit SHALL notify
watchers and then observers as usual. Other results SHALL leave that claim for release and keep observers silent.

#### Scenario: A mixed batch

- **WHEN** a batch contains a message at the limit and one below it
- **THEN** only the runnable message can be returned, and a confirmed failure uses cause ClaimLimit and the existing
  claim-limit reason with the observed lostClaims count
- **AND** the server's observers hear start then end only for that confirmed failure, never for a returned lease

#### Scenario: All claims reach the limit

- **WHEN** every claimed message reaches the limit
- **THEN** the result is Leased([]), with no user-handler execution
- **AND** no count of filtered messages is exposed and the call performs no inline commit retry or renewal

#### Scenario: Claim limit disabled

- **WHEN** max_lost_claims is None
- **THEN** high lostClaims alone does not prevent lease handoff

#### Scenario: Failed or uncertain claim-limit commit

- **WHEN** the single commit attempt fails, is refused or has an unknown result
- **THEN** the message is not handed out, the server keeps observers silent and attempts to release its epoch
- **AND** if no commit applied, the pending message remains at the limit for the next claimant to fail
- **AND** if the commit applied, release is a no-op and the watcher notification is not duplicated

#### Scenario: A watcher is notified once

- **WHEN** an at-limit failure commit succeeds, including when its reply is lost and cleanup subsequently runs
- **THEN** each eligible watcher receives the one stored failure notification for that event
- **AND** no retry-policy reschedule or user effect is added

#### Scenario: Local execution retains its policy

- **WHEN** a hosted local actor hits the claim limit through the ordinary executor
- **THEN** it retains the existing bounded commit-retry and abandonment behavior
- **AND** the external admission path's single-attempt-and-release rule does not change local execution
