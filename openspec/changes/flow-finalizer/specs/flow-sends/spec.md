## MODIFIED Requirements

### Requirement: Reserved flow behavior is explicit during rollout

This release SHALL preserve the ADR 0044 wire shape and enable requests and single/all/first reply waits with or without deadlines. It SHALL
enable Cancel to enter cancelling and support cleanup continuations/replies in that phase; repeated Cancel there is a no-op. Continue and Cancel in done/cancelled SHALL
do nothing and Reply in those states SHALL be ignored. Wait and request-send entries SHALL not replay through
plain-send/step/sleep APIs. Docs SHALL require all hosts/workers of a type upgraded before later features
are enabled, since equal contract hashes do not establish equal supported behavior.

#### Scenario: Reserved message and phase

- **WHEN** Continue, Reply or Cancel reaches cancelling state
- **THEN** Continue normalizes or executes cleanup, Reply routes to cleanup, and Cancel is a no-op
- **WHEN** Continue reaches structurally valid done/cancelled state
- **THEN** the state stays and no flow code runs

#### Scenario: Deployment choices

- **WHEN** a host adopts the changed flow contract
- **THEN** docs offer draining/archiving and deliberate registration reset, or a pinned old binary hosting old names
  alongside new binaries hosting new names with otherwise compatible contracts
- **AND** they do not promise automatic live migration, preservation of deleted mailboxes or a generic export/restore utility

#### Scenario: Reserved active wait

- **WHEN** a predecessor host without join support handles an eligible reply for a multi-request or first-mode open wait
- **THEN** that predecessor fails it permanently with a later-feature diagnostic and saves no reply or outcome

#### Scenario: Deadline behavior on an older host

- **WHEN** a predecessor host handles an eligible Reply for an open deadline wait
- **THEN** it may permanently refuse it despite an equal flow contract hash
- **AND** docs require all hosts/workers upgraded before enabling deadlines

