## MODIFIED Requirements

### Requirement: Reserved flow behavior is explicit during rollout

This release SHALL preserve the ADR 0044 wire shape and enable requests and single indefinite reply waits. It SHALL
permanently refuse Cancel and execution of cancelling state as unavailable features. Continue in done/cancelled SHALL
do nothing and Reply in those states SHALL be ignored. Wait and request-send entries SHALL not replay through
plain-send/step/sleep APIs. Joins and deadline waits SHALL remain unavailable. Docs SHALL require all hosts/workers of a type upgraded before later features
are enabled, since equal contract hashes do not establish equal supported behavior.

#### Scenario: Reserved message and phase

- **WHEN** Cancel reaches this release, or Continue/Reply reaches cancelling state
- **THEN** the message fails permanently with a later-release diagnostic and no effects
- **WHEN** Continue reaches structurally valid done/cancelled state
- **THEN** the state stays and no flow code runs

#### Scenario: Deployment choices

- **WHEN** a host adopts the changed flow contract
- **THEN** docs offer draining/archiving and deliberate registration reset, or a pinned old binary hosting old names
  alongside new binaries hosting new names with otherwise compatible contracts
- **AND** they do not promise automatic live migration, preservation of deleted mailboxes or a generic export/restore utility

#### Scenario: Reserved active wait

- **WHEN** an eligible reply would execute a saved open join or deadline wait
- **THEN** it fails permanently with a later-feature diagnostic and saves no reply or outcome
